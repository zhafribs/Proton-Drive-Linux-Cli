import contextlib
import importlib.util
import pathlib
import tempfile
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("proton_sync", ROOT / "scripts/proton-sync.py")
proton_sync = importlib.util.module_from_spec(spec)
spec.loader.exec_module(proton_sync)


def fake_list(rpath):
    return {
        "/my-files": [
            ("a", "/my-files/a", {"uid": "1", "type": "dir", "mtime": "", "size": 0})
        ],
        "/my-files/a": [
            (
                "a/b.txt",
                "/my-files/a/b.txt",
                {"uid": "2", "type": "file", "mtime": "2026-01-01T00:00:00Z", "size": 3},
            )
        ],
    }[rpath]


class AuditStateTests(unittest.TestCase):
    def test_two_chunks_complete_bfs_tree(self):
        state = proton_sync.AuditState("/my-files")
        state.next_chunk(fake_list, max_folders=1)
        self.assertFalse(state.complete)
        self.assertEqual(state.done, 1)
        self.assertIn("a", state.tree)

        state.next_chunk(fake_list, max_folders=1)
        self.assertTrue(state.complete)
        self.assertEqual(state.done, 2)
        self.assertEqual(state.total, 2)
        self.assertIn("a/b.txt", state.tree)

    def test_raw_cli_entries_are_parsed(self):
        state = proton_sync.AuditState("/my-files")
        raw = [{
            "uid": "1",
            "type": "folder",
            "name": {"ok": True, "value": "a"},
            "modificationTime": "2026-01-01T00:00:00Z",
            "activeRevision": {},
        }]
        state.next_chunk(lambda _rpath: raw, max_folders=1)
        self.assertEqual(state.tree["a"]["type"], "dir")
        self.assertFalse(state.complete)

    def test_health_reports_progress(self):
        state = proton_sync.AuditState("/my-files")
        state.next_chunk(fake_list, max_folders=1)
        health = state.health()
        self.assertEqual(health["audit_done"], 1)
        self.assertEqual(health["audit_total"], 2)
        self.assertGreaterEqual(health["audit_seconds"], 0)


class AuditFailureTests(unittest.TestCase):
    def test_permanent_lister_error_keeps_audit_pending(self):
        state = proton_sync.AuditState("/my-files")
        with patch.object(proton_sync, "AUDIT_PENDING", True), patch.object(
            proton_sync, "AUDIT_STATE", state
        ), patch.object(proton_sync, "cli_lock", return_value=contextlib.nullcontext()), patch.object(
            proton_sync, "probe_session"
        ), patch.object(
            proton_sync, "scan_remote_chunk", side_effect=proton_sync.CliError("boom")
        ), patch.object(
            proton_sync, "write_health"
        ) as write_health:
            complete = proton_sync.audit_tick()
            self.assertFalse(complete)
            self.assertTrue(proton_sync.AUDIT_PENDING)
            self.assertEqual(state.error, "boom")
            self.assertEqual(write_health.call_args.kwargs["audit_error"], "boom")


class ScopeRefreshTests(unittest.TestCase):
    def test_root_scope_refresh_is_supported(self):
        remote = {
            "old.txt": {"uid": "old", "type": "file", "size": 1, "mtime": "2026-01-01T00:00:00Z"},
            "old-folder": {"uid": "olddir", "type": "dir", "size": 0, "mtime": ""},
        }
        entries = [{
            "uid": "new",
            "type": "file",
            "name": {"ok": True, "value": "new.txt"},
            "modificationTime": "2026-01-02T00:00:00Z",
            "activeRevision": {"claimedSize": 2},
        }]
        with patch.object(proton_sync, "_list_folder", return_value=("/my-files", entries, None)):
            proton_sync.apply_scope(remote, "")
        self.assertIn("new.txt", remote)
        self.assertNotIn("old.txt", remote)
        self.assertNotIn("old-folder", remote)


class AuditFinishTests(unittest.TestCase):
    def test_complete_audit_swaps_and_saves_snapshot(self):
        state = proton_sync.AuditState("/my-files")
        state.next_chunk(fake_list, max_folders=1)
        state.next_chunk(fake_list, max_folders=1)
        self.assertTrue(state.complete)
        with patch.object(proton_sync, "AUDIT_STATE", state), patch.object(
            proton_sync, "AUDIT_PENDING", True
        ), patch.object(proton_sync, "FAST_VIEW", None), patch.object(
            proton_sync, "remote_fingerprint", return_value=[1, 2]
        ), patch.object(
            proton_sync, "save_snapshot"
        ) as save, patch.object(
            proton_sync, "write_health"
        ):
            self.assertTrue(proton_sync.finish_audit_if_ready())
            self.assertEqual(proton_sync.FAST_VIEW, state.tree)
            self.assertFalse(proton_sync.AUDIT_PENDING)
            self.assertIsNone(proton_sync.AUDIT_STATE)
            save.assert_called_once()


class AuditRequestTests(unittest.TestCase):
    def test_request_file_starts_audit_and_is_consumed(self):
        with tempfile.TemporaryDirectory() as tmp:
            request = pathlib.Path(tmp) / "audit-request"
            request.write_text("", encoding="utf-8")
            with patch.object(proton_sync, "AUDIT_REQUEST_FILE", request), patch.object(
                proton_sync, "FAST_START", True
            ), patch.object(proton_sync, "FAST_VIEW", {"a": {"type": "file"}}), patch.object(
                proton_sync, "AUDIT_PENDING", False
            ), patch.object(proton_sync, "AUDIT_STATE", None), patch.object(
                proton_sync, "write_health"
            ):
                self.assertTrue(proton_sync.handle_audit_request())
                self.assertFalse(request.exists())
                self.assertTrue(proton_sync.AUDIT_PENDING)
                self.assertIsInstance(proton_sync.AUDIT_STATE, proton_sync.AuditState)


class TouchedScopeTests(unittest.TestCase):
    def test_touched_scope_is_relisted_before_swap(self):
        state = proton_sync.AuditState("/my-files")
        state.tree["a"] = {"uid": "1", "type": "dir", "mtime": "", "size": 0}
        state.touched.add("a")
        calls = []
        with patch.object(
            proton_sync, "apply_scope", side_effect=lambda tree, rel: calls.append(rel)
        ):
            proton_sync.merge_touched_scopes(state)
        self.assertEqual(calls, ["a"])


if __name__ == "__main__":
    unittest.main()
