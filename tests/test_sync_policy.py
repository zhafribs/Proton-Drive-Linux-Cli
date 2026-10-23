import importlib.util
import pathlib
import time
import unittest

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

    def test_allows_download_and_directory_upload(self):
        actions = [
            ("dl", "remote.txt", {"type": "file"}),
            ("up", "newdir", {"type": "dir"}),
        ]
        allowed, held = proton_sync.hold_actions_for_audit(actions, {}, {})
        self.assertEqual(allowed, actions)
        self.assertEqual(held, 0)


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


class HealthPhaseTests(unittest.TestCase):
    def test_reports_phase_elapsed_and_extras(self):
        started = time.monotonic() - 0.5
        payload = proton_sync.health_phase("scan-remote", started, cycle_reason="startup")
        self.assertEqual(payload["phase"], "scan-remote")
        self.assertGreaterEqual(payload["phase_seconds"], 0.4)
        self.assertEqual(payload["cycle_reason"], "startup")


if __name__ == "__main__":
    unittest.main()
