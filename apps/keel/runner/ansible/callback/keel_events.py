# SPDX-License-Identifier: Apache-2.0
"""Keel's Ansible callback plugin: one ``keel_events`` JSON line per event (spec 055, T068).

The plugin is stdout-neutral: it prints nothing and leaves the stdout callback alone. Each
event is written as one JSON line to the file descriptor named in ``KEEL_EVENT_FD``; with the
variable unset (or the fd closed) the plugin does nothing. Enable it with
``ANSIBLE_CALLBACKS_ENABLED=keel_events``.

Line format, version 1 (parsed by ``keel-exec``'s ``events`` module)::

    {"v": 1, "event": "playbook_start", "playbook": ...}
    {"v": 1, "event": "play_start", "play": ...}
    {"v": 1, "event": "task_start", "task": ..., "task_id": ..., "action": ..., "handler": bool}
    {"v": 1, "event": "ok|changed|failed|unreachable|skipped", "host": ..., "task": ...,
     "task_id": ..., "action": ..., "ignore_errors": bool, "result": {...}}
    {"v": 1, "event": "stats", "stats": {host: {ok, changed, failures, unreachable, skipped,
     rescued, ignored}}}

A ``no_log`` result is written only as ``{"censored": ..., "changed": ...}``.

The event building is plain functions over duck-typed objects, so it is testable without
ansible-core installed (see test_keel_events.py).
"""

import json
import os

try:
    from ansible.plugins.callback import CallbackBase
except ImportError:  # pragma: no cover - exercised only where ansible-core is absent
    CallbackBase = object

LINE_VERSION = 1
CENSORED_MESSAGE = "the output has been hidden due to the fact that 'no_log: true' was specified for this result"
# A result line larger than this is replaced by a summary (the frame cap is 16 MiB).
MAX_LINE_BYTES = 1024 * 1024
MAX_FIELD_CHARS = 4096
STATS_KEYS = ("ok", "changed", "failures", "unreachable", "skipped", "rescued", "ignored")


def is_no_log(task, result):
    """Whether this result must be censored."""
    if getattr(task, "no_log", False) is True:
        return True
    if isinstance(result, dict):
        return result.get("_ansible_no_log") is True or "censored" in result
    return False


def censor_result(result):
    """The only view of a ``no_log`` result that may leave the process."""
    out = {"censored": CENSORED_MESSAGE}
    if isinstance(result, dict):
        if isinstance(result.get("censored"), str):
            out["censored"] = result["censored"]
        if isinstance(result.get("changed"), bool):
            out["changed"] = result["changed"]
    return out


def clean_result(result):
    """Drop ansible-internal keys and the module invocation (it echoes arguments)."""
    if not isinstance(result, dict):
        return {}
    return {k: v for k, v in result.items() if not k.startswith("_ansible") and k != "invocation"}


def _summary(result):
    """A small stand-in for a result too large to send."""
    out = {"truncated": True}
    for key in ("changed", "rc", "msg", "stderr"):
        value = result.get(key)
        if isinstance(value, str):
            value = value[:MAX_FIELD_CHARS]
        if value is not None:
            out[key] = value
    return out


def task_fields(task):
    return {
        "task": str(task.get_name()) if hasattr(task, "get_name") else str(getattr(task, "name", "")),
        "task_id": str(getattr(task, "_uuid", "")),
        "action": str(getattr(task, "action", "")),
    }


def build_task_start(task, handler=False):
    event = {"v": LINE_VERSION, "event": "task_start"}
    event.update(task_fields(task))
    event["handler"] = bool(handler)
    return event


def build_host_result(status, host, task, result, ignore_errors=False):
    """A per-host result event. ``status`` is ok, failed, unreachable or skipped; an ``ok``
    result that changed something is written as ``changed``."""
    if status == "ok" and isinstance(result, dict) and result.get("changed") is True:
        status = "changed"
    if is_no_log(task, result):
        body = censor_result(result)
    else:
        body = clean_result(result)
    event = {"v": LINE_VERSION, "event": status, "host": str(host)}
    event.update(task_fields(task))
    event["ignore_errors"] = bool(ignore_errors)
    event["result"] = body
    return event


def build_stats(stats):
    """The per-host summary from an ``AggregateStats``."""
    out = {}
    for host in sorted(getattr(stats, "processed", {}) or {}):
        summary = stats.summarize(host)
        out[str(host)] = {key: int(summary.get(key, 0)) for key in STATS_KEYS}
    return {"v": LINE_VERSION, "event": "stats", "stats": out}


def encode(event):
    """One JSON line, never larger than MAX_LINE_BYTES."""
    line = json.dumps(event, default=str, ensure_ascii=False) + "\n"
    data = line.encode("utf-8", "replace")
    if len(data) > MAX_LINE_BYTES and isinstance(event.get("result"), dict):
        event = dict(event, result=_summary(event["result"]))
        data = (json.dumps(event, default=str, ensure_ascii=False) + "\n").encode("utf-8", "replace")
    return data


class EventWriter:
    """Writes lines to a file descriptor; disables itself on the first error."""

    def __init__(self, fd):
        self.fd = fd

    @classmethod
    def from_env(cls, environ=None):
        value = (environ if environ is not None else os.environ).get("KEEL_EVENT_FD", "")
        try:
            fd = int(value)
        except ValueError:
            fd = None
        if fd is not None and fd < 0:
            fd = None
        return cls(fd)

    def write(self, event):
        if self.fd is None:
            return
        data = encode(event)
        try:
            while data:
                written = os.write(self.fd, data)
                data = data[written:]
        except OSError:
            # The runner went away; Ansible's own run must not fail because of it.
            self.fd = None


def _result_parts(result):
    """(host name, task, result dict) from a TaskResult (ansible-core < 2.19 private fields, or
    the public ones of 2.19+)."""
    host = getattr(result, "host", None) or getattr(result, "_host", None)
    task = getattr(result, "task", None) or getattr(result, "_task", None)
    body = getattr(result, "result", None)
    if body is None:
        body = getattr(result, "_result", {})
    name = host.get_name() if hasattr(host, "get_name") else str(host)
    return name, task, body


class CallbackModule(CallbackBase):
    CALLBACK_VERSION = 2.0
    CALLBACK_TYPE = "notification"
    CALLBACK_NAME = "keel_events"
    CALLBACK_NEEDS_ENABLED = True

    def __init__(self, *args, **kwargs):
        if CallbackBase is not object:
            super().__init__(*args, **kwargs)
        self._writer = EventWriter.from_env()

    def _host_result(self, status, result, ignore_errors=False):
        host, task, body = _result_parts(result)
        self._writer.write(build_host_result(status, host, task, body, ignore_errors))

    def v2_playbook_on_start(self, playbook):
        name = getattr(playbook, "_file_name", "")
        self._writer.write({"v": LINE_VERSION, "event": "playbook_start", "playbook": str(name)})

    def v2_playbook_on_play_start(self, play):
        name = play.get_name() if hasattr(play, "get_name") else ""
        self._writer.write({"v": LINE_VERSION, "event": "play_start", "play": str(name)})

    def v2_playbook_on_task_start(self, task, is_conditional):
        self._writer.write(build_task_start(task))

    def v2_playbook_on_handler_task_start(self, task):
        self._writer.write(build_task_start(task, handler=True))

    def v2_runner_on_ok(self, result):
        self._host_result("ok", result)

    def v2_runner_on_failed(self, result, ignore_errors=False):
        self._host_result("failed", result, ignore_errors)

    def v2_runner_on_unreachable(self, result):
        self._host_result("unreachable", result)

    def v2_runner_on_skipped(self, result):
        self._host_result("skipped", result)

    def v2_playbook_on_stats(self, stats):
        self._writer.write(build_stats(stats))
