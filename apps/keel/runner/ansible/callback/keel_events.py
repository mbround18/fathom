# SPDX-License-Identifier: Apache-2.0
"""Keel's Ansible callback plugin: emits one ``keel_events`` JSON line per task result.

Spec 055, Phase 1 placeholder. The line format and the parser (``keel-exec``'s ``events``
module) arrive together in a later phase.
"""

from ansible.plugins.callback import CallbackBase


class CallbackModule(CallbackBase):
    CALLBACK_VERSION = 2.0
    CALLBACK_TYPE = "notification"
    CALLBACK_NAME = "keel_events"
