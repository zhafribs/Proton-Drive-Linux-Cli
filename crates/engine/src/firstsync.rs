//! First download: read the cloud tree and pull it down in one overlapped pass.
//!
//! A fresh install used to wait for the whole remote walk before the first
//! byte moved (walk the tree, plan, then download). Here every folder is
//! dispatched to a transfer pool the moment it is listed, so downloads start
//! within seconds and run alongside the walk.
//!
//! Only used when nothing is on disk and the state file is empty, where every
//! remote path is simply a download - so streaming the plan is safe.

use crate::cli::{self, Cli};
use crate::filter::Filter;
use anyhow::Result;
use proton_core::snapshot::{RemoteEntry, RemoteTree};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};

/// Files per CLI call. One call per parent folder, like the planned transfers.
const MAX_BATCH: usize = 25;

pub struct FirstSync {
    pub tree: RemoteTree,
    pub downloaded: BTreeSet<String>,
    pub failed: BTreeSet<String>,
}

struct Batch {
    parent: String,
    rels: Vec<String>,
}

/// Walk the remote tree and download it as it is discovered.
pub fn download_all(
    cli: &Cli,
    remote_root: &str,
    local_root: &Path,
    filter: &Filter,
) -> Result<FirstSync> {
    let workers = cli::cfg_int("PROTON_SYNC_WORKERS", 16, 32);
    let transfers = cli::cfg_int("PROTON_SYNC_TRANSFERS", 8, 32);
    let retries = cli::cfg_int("PROTON_SYNC_RETRIES", 3, 10);
    let start_rpath = cli::remote_path(remote_root, "");

    cli::with_cli_lock(|| {
        let _ = cli.run(&["fs", "info", remote_root], true, Some(120), false);

        let (tx, rx) = mpsc::sync_channel::<Batch>(transfers * 4);
        let rx = Arc::new(Mutex::new(rx));
        let downloaded: Arc<Mutex<BTreeSet<String>>> = Arc::new(Mutex::new(BTreeSet::new()));
        let failed: Arc<Mutex<BTreeSet<String>>> = Arc::new(Mutex::new(BTreeSet::new()));
        let done = Arc::new(AtomicU64::new(0));

        let mut tree: Option<RemoteTree> = None;
        let mut scan_error: Option<anyhow::Error> = None;

        std::thread::scope(|s| {
            // transfers: drain batches while the walk keeps filling the queue
            for _ in 0..transfers {
                let rx = Arc::clone(&rx);
                let downloaded = Arc::clone(&downloaded);
                let failed = Arc::clone(&failed);
                let done = Arc::clone(&done);
                s.spawn(move || loop {
                    let batch = {
                        let guard = rx.lock().unwrap();
                        guard.recv()
                    };
                    let Ok(batch) = batch else { return };
                    let n = batch.rels.len() as u64;
                    let target = local_root.join(&batch.parent);
                    let target_s = target.to_string_lossy().into_owned();
                    let rpaths: Vec<String> = batch
                        .rels
                        .iter()
                        .map(|r| cli::remote_path(remote_root, r))
                        .collect();
                    let mut args: Vec<&str> = vec!["fs", "download"];
                    args.extend(rpaths.iter().map(String::as_str));
                    args.extend(["-f", "remove", "-d", "merge"]);
                    args.push(&target_s);

                    let mut res = cli.run(&args, false, Some(3600), false);
                    for attempt in 1..retries {
                        if res.is_ok() {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_secs(2 * attempt as u64));
                        res = cli.run(&args, false, Some(3600), false);
                    }
                    match res {
                        Ok(_) => {
                            for rel in &batch.rels {
                                eprintln!("DOWNLOAD {}", rel);
                            }
                            downloaded.lock().unwrap().extend(batch.rels);
                        }
                        Err(e) => {
                            eprintln!("FAILED download batch ({} path(s)): {}", n, e);
                            failed.lock().unwrap().extend(batch.rels);
                        }
                    }
                    let done = done.fetch_add(n, Ordering::Relaxed) + n;
                    crate::execute::report_progress(done);
                });
            }

            // The walk owns the last sender: when it finishes, the channel
            // closes and the transfer workers stop. (Borrowing a sender that
            // outlives the scope would leave them waiting forever.)
            let walk_tx = tx;
            let walk = s.spawn(move || -> Result<RemoteTree> {
                let mut tree = RemoteTree::new();
                let mut visited: HashSet<String> = HashSet::new();
                let mut rel_of: HashMap<String, String> = HashMap::new();
                rel_of.insert(start_rpath.clone(), String::new());
                let mut level = vec![start_rpath.clone()];

                while !level.is_empty() {
                    level.retain(|p| visited.insert(p.clone()));
                    let items: Vec<(String, String)> = level
                        .iter()
                        .map(|p| (p.clone(), rel_of.get(p).cloned().unwrap_or_default()))
                        .collect();
                    let listed = cli::run_parallel(items, workers, |(rpath, rel)| {
                        let mut entries = cli.run_json(&["fs", "list", &rpath], false);
                        for attempt in 1..retries {
                            if entries.is_ok() {
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_secs(2 * attempt as u64));
                            entries = cli.run_json(&["fs", "list", &rpath], false);
                        }
                        (rpath, rel, entries)
                    });

                    let mut next: Vec<String> = Vec::new();
                    for (rpath, rel, entries) in listed {
                        let Value::Array(arr) = entries? else { continue };
                        let mut by_parent: BTreeMap<String, Vec<String>> = BTreeMap::new();
                        for e in arr {
                            let Some(obj) = e.as_object() else { continue };
                            if !obj.contains_key("type") {
                                continue;
                            }
                            let typ = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            let name_obj = obj.get("name");
                            let ok = name_obj
                                .and_then(|n| n.get("ok"))
                                .and_then(|v| v.as_bool())
                                .unwrap_or(true);
                            let name = name_obj
                                .and_then(|n| n.get("value"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if !ok || name.is_empty() {
                                continue;
                            }
                            if Filter::skip_name(name) {
                                continue;
                            }
                            let child_rel = if rel.is_empty() {
                                name.to_string()
                            } else {
                                format!("{}/{}", rel, name)
                            };
                            if filter.excluded(&child_rel) {
                                continue;
                            }
                            let media =
                                obj.get("mediaType").and_then(|v| v.as_str()).unwrap_or("");
                            let size = obj
                                .get("activeRevision")
                                .and_then(|r| r.get("claimedSize"))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0);
                            if typ == "folder" {
                                if !filter.allowed_dir(&child_rel) {
                                    continue;
                                }
                                tree.insert(
                                    child_rel.clone(),
                                    RemoteEntry {
                                        uid: obj
                                            .get("uid")
                                            .and_then(|v| v.as_str())
                                            .map(str::to_string),
                                        kind: "dir".to_string(),
                                        mtime: obj
                                            .get("modificationTime")
                                            .and_then(|v| v.as_str())
                                            .map(str::to_string),
                                        size: Some(0),
                                        doc: false,
                                    },
                                );
                                // create it now: a batch may land in it shortly
                                let _ = fs::create_dir_all(local_root.join(&child_rel));
                                let child_rpath =
                                    format!("{}/{}", rpath, cli::escape(name));
                                rel_of.insert(child_rpath.clone(), child_rel);
                                next.push(child_rpath);
                            } else if typ == "file" {
                                if !filter.allowed_file(&child_rel) {
                                    continue;
                                }
                                let doc = media.starts_with("application/vnd.proton");
                                tree.insert(
                                    child_rel.clone(),
                                    RemoteEntry {
                                        uid: obj
                                            .get("uid")
                                            .and_then(|v| v.as_str())
                                            .map(str::to_string),
                                        kind: "file".to_string(),
                                        mtime: obj
                                            .get("modificationTime")
                                            .and_then(|v| v.as_str())
                                            .map(str::to_string),
                                        size: Some(size),
                                        doc,
                                    },
                                );
                                // Proton Docs/Sheets cannot be downloaded
                                if !doc {
                                    by_parent.entry(rel.clone()).or_default().push(child_rel);
                                }
                            }
                        }
                        for (parent, rels) in by_parent {
                            for chunk in rels.chunks(MAX_BATCH) {
                                let batch = Batch {
                                    parent: parent.clone(),
                                    rels: chunk.to_vec(),
                                };
                                if send_until_open(&walk_tx, batch).is_err() {
                                    return Ok(tree);
                                }
                            }
                        }
                    }
                    level = next;
                }
                Ok(tree)
            });

            match walk.join() {
                Ok(Ok(t)) => tree = Some(t),
                Ok(Err(e)) => scan_error = Some(e),
                Err(_) => scan_error = Some(anyhow::anyhow!("first-sync walk panicked")),
            }
        });

        if let Some(e) = scan_error {
            return Err(e);
        }
        let tree = tree.unwrap_or_default();
        let downloaded = Arc::try_unwrap(downloaded)
            .map(|m| m.into_inner().unwrap())
            .unwrap_or_default();
        let failed = Arc::try_unwrap(failed)
            .map(|m| m.into_inner().unwrap())
            .unwrap_or_default();
        Ok(FirstSync {
            tree,
            downloaded,
            failed,
        })
    })
}

/// Send a batch, retrying while the queue is full (never drop work).
fn send_until_open(tx: &SyncSender<Batch>, mut batch: Batch) -> std::result::Result<(), ()> {
    loop {
        match tx.try_send(batch) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(b)) => {
                batch = b;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return Err(()),
        }
    }
}
