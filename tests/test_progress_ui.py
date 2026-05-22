# ============================================================================
# Tests: progress_ui.py
# ============================================================================
from __future__ import annotations

import io
import sys
from pathlib import Path

import pytest

from src.progress_ui import (
    Phase,
    PhaseStatus,
    ProgressUI,
    _bar,
    _elapsed_str,
    _visible_len,
)


# ---------------------------------------------------------------------------
# Phase data model
# ---------------------------------------------------------------------------
class TestPhase:
    def test_defaults(self):
        p = Phase(name='test')
        assert p.status == PhaseStatus.PENDING
        assert p.message == ''
        assert p.elapsed_ms == 0.0
        assert p.detail == ''
        assert p.error == ''

    def test_mutable_fields(self):
        p = Phase(name='build')
        p.status = PhaseStatus.RUNNING
        p.message = 'compiling...'
        p.elapsed_ms = 123.4
        assert p.status == PhaseStatus.RUNNING
        assert p.message == 'compiling...'
        assert p.elapsed_ms == 123.4


# ---------------------------------------------------------------------------
# Bar drawing
# ---------------------------------------------------------------------------
class TestBar:
    def test_empty(self):
        bar = _bar(0, 6)
        assert '0%' in bar

    def test_half(self):
        bar = _bar(3, 6)
        assert '50%' in bar

    def test_full(self):
        bar = _bar(6, 6)
        assert '100%' in bar

    def test_zero_total(self):
        bar = _bar(0, 0)
        # ratio clamped to 1.0, but bar displays 0%
        assert '0%' in bar or '100%' in bar


# ---------------------------------------------------------------------------
# Elapsed formatting
# ---------------------------------------------------------------------------
class TestElapsedStr:
    def test_milliseconds(self):
        assert '500ms' in _elapsed_str(500)

    def test_seconds(self):
        assert _elapsed_str(1500) == '  1.5s'

    def test_zero(self):
        assert _elapsed_str(0) == '    0ms'


# ---------------------------------------------------------------------------
# Visible length
# ---------------------------------------------------------------------------
class TestVisibleLen:
    def test_plain(self):
        assert _visible_len('hello') == 5

    def test_ansi(self):
        assert _visible_len('\033[32mhello\033[0m') == 5

    def test_empty(self):
        assert _visible_len('') == 0


# ---------------------------------------------------------------------------
# ProgressUI — structure & lifecycle
# ---------------------------------------------------------------------------
class TestProgressUI:
    def test_add_phase_returns_self(self):
        ui = ProgressUI('Test')
        result = ui.add_phase('a', 'Phase A')
        assert result is ui
        assert len(ui.phases) == 1

    def test_phase_lookup(self):
        ui = ProgressUI('Test')
        ui.add_phase('ctx', 'Context')
        ui.add_phase('cmd', 'Commands')
        assert ui.phase('ctx').name == 'Context'
        assert ui.phase('cmd').name == 'Commands'

    def test_multiple_phases(self):
        ui = ProgressUI('Test')
        ui.add_phase('a', 'A').add_phase('b', 'B').add_phase('c', 'C')
        assert len(ui.phases) == 3

    def test_extract_log_data(self):
        ui = ProgressUI('Test')
        ui.add_phase('a', 'Alpha').add_phase('b', 'Beta')
        ui.phase('a').status = PhaseStatus.COMPLETED
        ui.phase('a').elapsed_ms = 10.0
        ui.phase('a').detail = 'done'
        ui.phase('b').status = PhaseStatus.FAILED
        ui.phase('b').error = 'something broke'

        data = ui.extract_log_data()
        assert len(data) == 2
        assert data[0] == {
            'phase': 'Alpha', 'status': 'completed',
            'elapsed_ms': 10.0, 'message': '', 'detail': 'done', 'error': '',
        }
        assert data[1]['status'] == 'failed'
        assert data[1]['error'] == 'something broke'


# ---------------------------------------------------------------------------
# ProgressUI — render (captured output)
# ---------------------------------------------------------------------------
class TestProgressUIRender:
    def test_header_renders(self):
        ui = ProgressUI('Bootstrap')
        ui.add_phase('a', 'Phase A')

        buf = io.StringIO()
        old_stdout = sys.stdout
        sys.stdout = buf
        try:
            ui.start()
            output = buf.getvalue()
        finally:
            sys.stdout = old_stdout

        assert 'Bootstrap' in output

    def test_footer_summary(self):
        ui = ProgressUI('Test Summary')
        ui.add_phase('a', 'A').add_phase('b', 'B').add_phase('c', 'C')
        ui.phase('a').status = PhaseStatus.COMPLETED
        ui.phase('a').elapsed_ms = 100
        ui.phase('b').status = PhaseStatus.COMPLETED
        ui.phase('b').elapsed_ms = 200
        ui.phase('c').status = PhaseStatus.FAILED
        ui.phase('c').elapsed_ms = 50
        ui.phase('c').error = 'connection refused'

        buf = io.StringIO()
        old_stdout = sys.stdout
        sys.stdout = buf
        try:
            ui.finish('All done!')
            output = buf.getvalue()
        finally:
            sys.stdout = old_stdout

        assert '2/3' in output
        assert 'Failed' in output
        assert 'All done!' in output

    def test_compact_render_no_phases(self):
        """Compact render should not crash with no phases."""
        ui = ProgressUI('Empty')
        # Should not raise
        ui._render_compact(None)

    def test_full_render_all_completed(self):
        ui = ProgressUI('Full')
        ui.add_phase('a', 'Alpha').add_phase('b', 'Beta')
        ui.phase('a').status = PhaseStatus.COMPLETED
        ui.phase('b').status = PhaseStatus.COMPLETED

        buf = io.StringIO()
        old_stdout = sys.stdout
        sys.stdout = buf
        try:
            ui._render_full()
            output = buf.getvalue()
        finally:
            sys.stdout = old_stdout

        # Both phases should appear
        assert 'Alpha' in output
        assert 'Beta' in output

    def test_set_compact_toggle(self):
        ui = ProgressUI('Toggle')
        assert ui._compact is True
        ui.set_compact(False)
        assert ui._compact is False
        ui.set_compact(True)
        assert ui._compact is True

    def test_on_complete_callback(self):
        called = []

        def cb(ui: ProgressUI) -> None:
            called.append(ui.title)

        ui = ProgressUI('Callback Test')
        ui.set_on_complete(cb)
        ui.finish()
        assert called == ['Callback Test']
