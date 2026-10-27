# SPDX-License-Identifier: Apache-2.0
"""Unit tests for the keel_events callback (spec 055, T062/T068).

Run with: python3 -m unittest discover -s apps/keel/runner/ansible/callback

The event-building tests use small fakes and need no ansible-core. The test that loads the plugin
through ansible's CallbackBase is skipped when ansible-core is not installed.
"""

import contextlib
import io
import json
import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import keel_events  # noqa: E402

try:
    import ansible  # noqa: F401

    HAVE_ANSIBLE = True
except ImportError:
    HAVE_ANSIBLE = False


class FakeHost:
    def __init__(self, name):
        self.name = name

    def get_name(self):
        return self.name


class FakeTask:
    def __init__(self, name, action="ansible.builtin.command", uuid="t1", no_log=False):
        self.name = name
        self.action = action
        self._uuid = uuid
        self.no_log = no_log

    def get_name(self):
        return self.name


class FakeResult:
    """The pre-2.19 TaskResult shape (private fields)."""

    def __init__(self, host, task, result):
        self._host = FakeHost(host)
        self._task = task
        self._result = result


class FakeStats:
    def __init__(self, per_host):
        self.processed = {h: 1 for h in per_host}
        self._per_host = per_host

    def summarize(self, host):
        return self._per_host[host]


class FakePlay:
    def get_name(self):
        return "Base hosts"


class FakePlaybook:
    _file_name = "site.yml"


def plugin_writing_to_pipe():
    read_fd, write_fd = os.pipe()
    old = os.environ.get("KEEL_EVENT_FD")
    os.environ["KEEL_EVENT_FD"] = str(write_fd)
    try:
        plugin = keel_events.CallbackModule()
    finally:
        if old is None:
            del os.environ["KEEL_EVENT_FD"]
        else:
            os.environ["KEEL_EVENT_FD"] = old
    return plugin, read_fd, write_fd


def read_lines(read_fd, write_fd):
    os.close(write_fd)
    chunks = []
    while True:
        data = os.read(read_fd, 65536)
        if not data:
            break
        chunks.append(data)
    os.close(read_fd)
    return [json.loads(line) for line in b"".join(chunks).decode().splitlines()]


SECRET = "hunter2-very-secret"


class NoLogTest(unittest.TestCase):
    def test_no_log_task_is_written_only_as_censored(self):
        task = FakeTask("Set password", "ansible.builtin.user", no_log=True)
        result = {"changed": True, "password": SECRET, "invocation": {"module_args": {"password": SECRET}}}
        event = keel_events.build_host_result("ok", "web-1", task, result)
        self.assertEqual(event["event"], "changed")
        self.assertEqual(set(event["result"]), {"censored", "changed"})
        self.assertNotIn(SECRET, json.dumps(event))

    def test_result_already_censored_by_ansible_stays_censored(self):
        task = FakeTask("Set password")
        result = {"censored": "hidden", "changed": False, "_ansible_no_log": True, "stdout": SECRET}
        event = keel_events.build_host_result("failed", "web-1", task, result)
        self.assertEqual(event["result"], {"censored": "hidden", "changed": False})
        self.assertNotIn(SECRET, json.dumps(event))

    def test_ansible_no_log_flag_alone_censors(self):
        event = keel_events.build_host_result(
            "failed", "web-1", FakeTask("t"), {"_ansible_no_log": True, "msg": "token=" + SECRET}
        )
        self.assertIn("censored", event["result"])
        self.assertNotIn(SECRET, json.dumps(event))

    def test_no_log_through_the_plugin_and_the_fd(self):
        plugin, r, w = plugin_writing_to_pipe()
        task = FakeTask("Set password", no_log=True)
        plugin.v2_runner_on_failed(FakeResult("web-1", task, {"msg": SECRET, "rc": 1}), ignore_errors=False)
        lines = read_lines(r, w)
        self.assertEqual(len(lines), 1)
        self.assertEqual(lines[0]["event"], "failed")
        self.assertEqual(list(lines[0]["result"]), ["censored"])
        self.assertNotIn(SECRET, json.dumps(lines))


class EventTest(unittest.TestCase):
    def test_a_whole_playbook_writes_one_line_per_event(self):
        plugin, r, w = plugin_writing_to_pipe()
        task = FakeTask("Install packages", "ansible.builtin.apt", "uuid-1")
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            plugin.v2_playbook_on_start(FakePlaybook())
            plugin.v2_playbook_on_play_start(FakePlay())
            plugin.v2_playbook_on_task_start(task, False)
            plugin.v2_runner_on_ok(FakeResult("web-1", task, {"changed": False, "_ansible_verbose_always": True}))
            plugin.v2_runner_on_ok(FakeResult("web-2", task, {"changed": True}))
            plugin.v2_runner_on_failed(FakeResult("web-3", task, {"rc": 3, "msg": "boom"}), ignore_errors=True)
            plugin.v2_runner_on_unreachable(FakeResult("db-1", task, {"unreachable": True, "msg": "ssh"}))
            plugin.v2_runner_on_skipped(FakeResult("web-4", task, {"skip_reason": "no"}))
            plugin.v2_playbook_on_handler_task_start(FakeTask("restart", uuid="uuid-2"))
            stats = {"ok": 1, "changed": 1, "failures": 0, "unreachable": 0, "skipped": 0, "rescued": 0, "ignored": 0}
            plugin.v2_playbook_on_stats(FakeStats({"web-1": stats}))
        self.assertEqual(out.getvalue(), "", "stdout-neutral")
        lines = read_lines(r, w)
        self.assertEqual(
            [line["event"] for line in lines],
            ["playbook_start", "play_start", "task_start", "ok", "changed", "failed", "unreachable", "skipped",
             "task_start", "stats"],
        )
        self.assertTrue(all(line["v"] == 1 for line in lines))
        self.assertEqual(lines[0]["playbook"], "site.yml")
        self.assertEqual(lines[2], {"v": 1, "event": "task_start", "task": "Install packages", "task_id": "uuid-1",
                                    "action": "ansible.builtin.apt", "handler": False})
        self.assertEqual(lines[3]["result"], {"changed": False}, "ansible-internal keys dropped")
        self.assertTrue(lines[5]["ignore_errors"])
        self.assertEqual(lines[5]["result"]["rc"], 3)
        self.assertTrue(lines[8]["handler"])
        self.assertEqual(lines[9]["stats"], {"web-1": stats})

    def test_invocation_is_never_written(self):
        event = keel_events.build_host_result(
            "ok", "h", FakeTask("t"), {"invocation": {"module_args": {"api_key": SECRET}}}
        )
        self.assertNotIn(SECRET, json.dumps(event))

    def test_oversized_results_are_summarised(self):
        event = keel_events.build_host_result("failed", "h", FakeTask("t"), {"stdout": "x" * (2 * 1024 * 1024), "rc": 2})
        data = keel_events.encode(event)
        self.assertLessEqual(len(data), keel_events.MAX_LINE_BYTES)
        self.assertEqual(json.loads(data)["result"], {"truncated": True, "rc": 2})

    def test_unset_or_bad_fd_disables_writing(self):
        self.assertIsNone(keel_events.EventWriter.from_env({}).fd)
        self.assertIsNone(keel_events.EventWriter.from_env({"KEEL_EVENT_FD": "stdout"}).fd)
        writer = keel_events.EventWriter.from_env({"KEEL_EVENT_FD": "-1"})
        writer.write({"v": 1})  # no error
        r, w = os.pipe()
        os.close(r)
        os.close(w)
        closed = keel_events.EventWriter(w)
        closed.write({"v": 1, "event": "play_start", "play": "p"})  # a closed fd does not raise
        self.assertIsNone(closed.fd)


@unittest.skipUnless(HAVE_ANSIBLE, "ansible-core is not installed; the CallbackBase integration test needs it")
class AnsibleIntegrationTest(unittest.TestCase):
    def test_plugin_loads_on_callback_base(self):
        from ansible.plugins.callback import CallbackBase

        self.assertTrue(issubclass(keel_events.CallbackModule, CallbackBase))
        self.assertEqual(keel_events.CallbackModule.CALLBACK_NAME, "keel_events")


if __name__ == "__main__":
    unittest.main()
