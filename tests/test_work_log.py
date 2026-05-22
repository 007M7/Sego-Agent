# ============================================================================
# Tests: work_log.py
# ============================================================================
from __future__ import annotations

import json
import os
import tempfile
from pathlib import Path

import pytest

from src.work_log import LogEntry, WorkLog, DEFAULT_LOG_DIR


class TestLogEntry:
    def test_to_dict(self):
        entry = LogEntry(timestamp='2026-01-01T00:00:00', level='phase',
                         section='context', data={'key': 'val'})
        d = entry.to_dict()
        assert d == {
            'timestamp': '2026-01-01T00:00:00',
            'level': 'phase',
            'section': 'context',
            'data': {'key': 'val'},
        }

    def test_default_data(self):
        entry = LogEntry(timestamp='ts', level='error', section='exec')
        assert entry.data == {}


class TestWorkLog:
    def test_phase_builder(self):
        wl = WorkLog()
        wl.phase('context', status='completed', elapsed_ms=12.3,
                 detail='37 Python files', message='ok')
        assert wl._phase_count == 1
        assert wl._completed_phases == 1
        assert wl._failed_phases == 0
        assert not wl.has_errors

    def test_phase_failed(self):
        wl = WorkLog()
        wl.phase('routing', status='failed', error='timeout')
        assert wl._failed_phases == 1
        assert wl._errors == ['routing: timeout']
        assert wl.has_errors

    def test_routing_builder(self):
        wl = WorkLog()
        wl.routing('routing', [{'kind': 'command', 'name': 'Bash', 'score': 5}])
        assert len(wl._entries) == 1

    def test_execution_builder(self):
        wl = WorkLog()
        wl.execution('exec', kind='command', name='Bash',
                     message='handled', handled=True)
        assert len(wl._entries) == 1

    def test_turn_builder(self):
        wl = WorkLog()
        wl.turn('persist', prompt='hello', output='world',
                stop_reason='completed', usage_input=10, usage_output=5)
        assert len(wl._entries) == 1

    def test_summary_builder(self):
        wl = WorkLog()
        wl.summary('finish', total_ms=123, session_path='/tmp/s.json')
        assert len(wl._entries) == 1

    def test_detail_builder(self):
        wl = WorkLog()
        wl.detail('extra', key='value')
        assert len(wl._entries) == 1

    def test_error_builder(self):
        wl = WorkLog()
        wl.error('persist', 'disk full', code=28)
        assert len(wl._entries) == 1
        assert wl._errors == ['persist: disk full']
        assert wl.has_errors

    def test_flush_writes_jsonl(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            path = Path(tmpdir) / 'test.jsonl'
            wl = WorkLog(path=path)
            wl.phase('context', detail='ok')
            wl.routing('routing', [{'kind': 'tool', 'name': 'Read', 'score': 3}])

            flushed = wl.flush()
            assert flushed == path
            assert path.exists()

            lines = path.read_text().strip().split('\n')
            assert len(lines) == 2
            for line in lines:
                obj = json.loads(line)
                assert 'timestamp' in obj
                assert 'level' in obj
                assert 'section' in obj

    def test_to_dict(self):
        wl = WorkLog()
        wl.phase('a', status='completed')
        wl.phase('b', status='completed')
        wl.phase('c', status='failed', error='oops')
        d = wl.to_dict()
        assert d['phase_count'] == 3
        assert d['completed_phases'] == 2
        assert d['failed_phases'] == 1
        assert d['entry_count'] == 3
        assert 'session_path' in d
        assert 'elapsed_sec' in d

    def test_summary_line(self):
        wl = WorkLog()
        wl.phase('a')
        wl.phase('b')
        line = wl.summary_line
        assert '✅' in line
        assert '2/2 phases' in line

    def test_summary_line_with_errors(self):
        wl = WorkLog()
        wl.phase('a', status='failed', error='boom')
        line = wl.summary_line
        assert '❌' in line
        assert 'ERRORS' in line

    def test_empty_flush_creates_parent_dir(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            deep = Path(tmpdir) / 'nested' / 'logs' / 'empty.jsonl'
            wl = WorkLog(path=deep)
            result = wl.flush()
            assert result == deep
            # File should exist even if no entries
            assert deep.parent.exists()

    def test_chained_builders(self):
        wl = WorkLog()
        result = (
            wl.phase('a')
              .phase('b')
              .routing('r', [])
              .execution('e')
              .turn('t')
        )
        assert result is wl
        assert len(wl._entries) == 5
