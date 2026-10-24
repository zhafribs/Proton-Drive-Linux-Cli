import contextlib
import importlib.util
import pathlib
import threading
import time
import types
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("proton_sync", ROOT / "scripts/proton-sync.py")
proton_sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proton_sync)


class SnapshotUsableTests(unittest.TestCase):
    def test_missing_snapshot_is_not_usable(self):
        self.assertEqual(proton_sync.snapshot_usable(None, {}), (False, "no snapshot"))

    def test_valid_snapshot_with_state_is_usable(self):
        snapshot = {
            "sig": "signature",
            "fp": [1, 1],
            "tree": {"a.txt": {"uid": "u1", "type": "file", "size": 1, "mtime": ""}},
        }
        state = {"a.txt": {"r_mtime": "2026-01-01T00:00:00Z"}}
        usable, reason = proton_sync.snapshot_usable(snapshot, state)
        self.assertTrue(usable, reason)

    def test_snapshot_with_entries_and_empty_state_is_not_usable(self):
        snapshot = {
            "sig": "signature",
            "fp": [1, 1],
            "tree": {"a.txt": {"uid": "u1", "type": "file", "size": 1, "mtime": ""}},
        }
        usable, reason = proton_sync.snapshot_usable(snapshot, {})
        self.assertFalse(usable)
        self.assertTrue(reason)

    def test_snapshot_missing_required_parts_is_not_usable(self):
        usable, reason = proton_sync.snapshot_usable({"tree": {}}, {})
        self.assertFalse(usable)
        self.assertTrue(reason)


class AuditHoldTests(unittest.TestCase):
    def test_holds_deletes_moves_and_conflicts(self):
        actions = [
            ("trash_remote", "old.txt", {}),
            ("trash_local", "old2.txt", {}),
            ("mv_remote", "new.txt", ("old.txt", "new.txt")),
            ("mv_local", "new2.txt", ("old2.txt", "new2.txt")),
            ("conflict", "conf.txt", ({}, {})),
        ]
        allowed, held = proton_sync.hold_actions_for_audit(actions, {}, {})
        self.assertEqual(allowed, [])
        self.assertEqual(held, len(actions))

    def test_holds_existing_file_edit(self):
        actions = [("up", "edit.txt", {"type": "file"})]
        state = {"edit.txt": {"r_mtime": "2026-01-01T00:00:00Z"}}
        allowed, held = proton_sync.hold_actions_for_audit(actions, {}, state)
        self.assertEqual(allowed, [])
        self.assertEqual(held, 1)

    def test_allows_new_file_upload(self):
        actions = [("up", "new.txt", {"type": "file"})]
        allowed, held = proton_sync.hold_actions_for_audit(actions, {}, {})
        self.assertEqual(allowed, actions)
        self.assertEqual(held, 0)

    def test_holds_existing_remote_file_without_state(self):
        remote = {"known.txt": {"uid": "u1", "type": "file", "size": 1, "mtime": "x"}}
        actions = [("up", "known.txt", {"type": "file"})]
        allowed, held = proton_sync.hold_actions_for_audit(actions, remote, {})
        self.assertEqual(allowed, [])
        self.assertEqual(held, 1)

    def test_allows_download_and_directory_upload(self):
        actions = [
            ("dl", "remote.txt", {"type": "file"}),
            ("up", "newdir", {"type": "dir"}),
        ]
        allowed, held = proton_sync.hold_actions_for_audit(actions, {}, {})
        self.assertEqual(allowed, actions)
        self.assertEqual(held, 0)


class HeldStateTests(unittest.TestCase):
    def test_refresh_state_preserves_held_paths(self):
        old = {"edit.txt": {
            "l_mtime_ns": 1,
            "l_size": 3,
            "r_mtime": "2026-01-01T00:00:00Z",
            "r_uid": "u1",
        }}
        remote = {"edit.txt": {
            "uid": "u1", "type": "file", "size": 3, "mtime": "2026-01-01T00:00:00Z",
        }}
        local = {"edit.txt": {"type": "file", "mtime_ns": 2, "size": 4}}
        saved = proton_sync.refresh_state(
            remote, local, old, set(), set(), planned_local=local, preserve={"edit.txt"}
        )
        self.assertEqual(saved["edit.txt"], old["edit.txt"])


class TransferBatchTests(unittest.TestCase):
    def test_groups_uploads_by_parent(self):
        actions = [
            ("up", "a/one.txt", {"type": "file"}),
            ("up", "a/two.txt", {"type": "file"}),
            ("up", "b/three.txt", {"type": "file"}),
        ]
        batches = proton_sync.group_transfer_batches(actions)
        self.assertEqual(len(batches), 2)
        by_parent = {batch["parent"]: batch for batch in batches}
        self.assertEqual(by_parent["a"]["uploads"], ["a/one.txt", "a/two.txt"])
        self.assertEqual(by_parent["a"]["downloads"], [])
        self.assertEqual(by_parent["b"]["uploads"], ["b/three.txt"])

    def test_keeps_upload_and_download_batches_separate(self):
        actions = [
            ("up", "a/one.txt", {"type": "file"}),
            ("dl", "a/remote.txt", {"type": "file"}),
        ]
        batches = proton_sync.group_transfer_batches(actions)
        self.assertEqual(len(batches), 2)
        self.assertTrue(all(not (batch["uploads"] and batch["downloads"]) for batch in batches))


class AdaptiveWorkerTests(unittest.TestCase):
    def test_uses_ram_budget(self):
        self.assertEqual(proton_sync.adaptive_worker_limit(12, 1024), 7)

    def test_unknown_memory_uses_configured_value(self):
        self.assertEqual(proton_sync.adaptive_worker_limit(12, None), 12)

    def test_rate_limit_lowers_and_clean_chunks_raise(self):
        self.assertEqual(proton_sync.adjust_worker_limit(8, True, 0), 6)
        self.assertEqual(proton_sync.adjust_worker_limit(6, False, 2), 6)
        self.assertEqual(proton_sync.adjust_worker_limit(6, False, 3), 7)
        self.assertEqual(proton_sync.adjust_worker_limit(16, False, 3), 16)


class RegressionGuardTests(unittest.TestCase):
    def test_worker_and_transfer_limits_stay_safe(self):
        self.assertGreaterEqual(proton_sync.SCAN_WORKERS, 1)
        self.assertLessEqual(proton_sync.SCAN_WORKERS, 16)
        self.assertGreaterEqual(proton_sync.TRANSFERS, 1)
        self.assertLessEqual(proton_sync.TRANSFERS, 10)
        self.assertGreaterEqual(proton_sync.AUDIT_CHUNK_FOLDERS, 1)


class TransferSchedulerTests(unittest.TestCase):
    def test_caps_concurrency_at_max_workers(self):
        lock = threading.Lock()
        active = 0
        peak = 0

        def fake(parent, rels, locked):
            nonlocal active, peak
            with lock:
                active += 1
                peak = max(peak, active)
            time.sleep(0.02)
            with lock:
                active -= 1

        batches = [
            {"parent": str(i), "uploads": [f"dir-{i}/file-{i}"], "downloads": []}
            for i in range(10)
        ]
        with patch.object(proton_sync, "cli_lock", return_value=contextlib.nullcontext()), patch.object(
            proton_sync, "probe_session"
        ), patch.object(proton_sync, "write_health"):
            failed, done = proton_sync.run_transfer_batches(batches, fake, fake, 4)
        self.assertLessEqual(peak, 4)
        self.assertEqual(failed, set())
        self.assertEqual(done, {f"dir-{i}/file-{i}" for i in range(10)})

    def test_rate_limit_backs_off_transfer_limit(self):
        old_limit = proton_sync.TRANSFER_LIMIT
        old_transfers = proton_sync.TRANSFERS
        proton_sync.TRANSFER_LIMIT = 8
        proton_sync.TRANSFERS = 10

        def fake(parent, rels, locked):
            raise proton_sync.CliError("429 too many requests")

        batches = [{"parent": "a", "uploads": ["a/one"], "downloads": []}]
        try:
            with patch.object(proton_sync, "cli_lock", return_value=contextlib.nullcontext()), patch.object(
                proton_sync, "probe_session"
            ), patch.object(proton_sync, "write_health"):
                failed, done = proton_sync.run_transfer_batches(batches, fake, fake, 4)
            self.assertEqual(failed, {"a/one"})
            self.assertEqual(done, set())
            self.assertEqual(proton_sync.TRANSFER_LIMIT, 6)
        finally:
            proton_sync.TRANSFER_LIMIT = old_limit
            proton_sync.TRANSFERS = old_transfers


class HealthPhaseTests(unittest.TestCase):
    def test_reports_phase_elapsed_and_extras(self):
        started = time.monotonic() - 0.5
        payload = proton_sync.health_phase("scan-remote", started, cycle_reason="startup")
        self.assertEqual(payload["phase"], "scan-remote")
        self.assertGreaterEqual(payload["phase_seconds"], 0.4)
        self.assertEqual(payload["cycle_reason"], "startup")


class FastViewTests(unittest.TestCase):
    def setUp(self):
        self._saved = (
            proton_sync.FAST_VIEW,
            proton_sync.AUDIT_PENDING,
            proton_sync.AUDIT_STATE,
            proton_sync.FAST_STARTED,
        )
        proton_sync.FAST_VIEW = None
        proton_sync.AUDIT_PENDING = False
        proton_sync.AUDIT_STATE = None
        proton_sync.FAST_STARTED = False

    def tearDown(self):
        (
            proton_sync.FAST_VIEW,
            proton_sync.AUDIT_PENDING,
            proton_sync.AUDIT_STATE,
            proton_sync.FAST_STARTED,
        ) = self._saved

    @patch.object(proton_sync, "write_health")
    @patch.object(proton_sync, "load_state")
    @patch.object(proton_sync, "load_snapshot")
    def test_starts_from_valid_snapshot(self, load_snapshot, load_state, write_health):
        tree = {"a.txt": {"uid": "u1", "type": "file", "size": 1, "mtime": ""}}
        load_snapshot.return_value = {
            "sig": "sig",
            "fp": [1, 1],
            "ts": time.time() - 10,
            "tree": tree,
        }
        load_state.return_value = {"a.txt": {"r_mtime": "2026-01-01T00:00:00Z"}}
        self.assertTrue(proton_sync.start_fast_view())
        self.assertEqual(proton_sync.FAST_VIEW, tree)
        self.assertTrue(proton_sync.AUDIT_PENDING)
        write_health.assert_called_once()

    @patch.object(proton_sync, "write_health")
    @patch.object(proton_sync, "load_state", return_value={})
    @patch.object(proton_sync, "load_snapshot")
    def test_rejects_snapshot_without_state(self, load_snapshot, _load_state, _write_health):
        load_snapshot.return_value = {
            "sig": "sig",
            "fp": [1, 1],
            "ts": time.time(),
            "tree": {"a.txt": {"uid": "u1", "type": "file", "size": 1, "mtime": ""}},
        }
        self.assertFalse(proton_sync.start_fast_view())
        self.assertIsNone(proton_sync.FAST_VIEW)
        self.assertFalse(proton_sync.AUDIT_PENDING)

    def test_fast_cycle_uses_cached_view(self):
        proton_sync.FAST_VIEW = {"a.txt": {"uid": "u1", "type": "file", "size": 1, "mtime": ""}}
        remote, reason = proton_sync.fast_cycle_remote(None)
        self.assertIn("a.txt", remote)
        self.assertEqual(reason, "fast")

    def test_push_scope_is_queued_for_the_audit_swap(self):
        proton_sync.FAST_VIEW = {"a/x.txt": {"uid": "u1", "type": "file", "size": 1, "mtime": ""}}
        proton_sync.AUDIT_PENDING = True
        state = proton_sync.AuditState("/my-files")
        proton_sync.AUDIT_STATE = state
        push = types.SimpleNamespace(volume_id="v", root_uid="r")
        with patch.object(proton_sync, "PUSH", push), patch.object(
            proton_sync, "resolve_push_scopes", return_value=(["a"], 0)
        ), patch.object(proton_sync, "apply_scope"):
            _remote, reason = proton_sync.fast_cycle_remote({"parent"})
        self.assertEqual(reason, "push")
        self.assertEqual(state.touched, {"a"})


if __name__ == "__main__":
    unittest.main()
