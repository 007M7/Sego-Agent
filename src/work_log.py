# ============================================================================
# Sego Agent — Work Log
# Structured, JSONL-based work-log persistence.
# Receives detailed execution data (phase details, routed matches, execution
# messages, turn results) and writes them to disk.
# The ProgressUI stays clean; all verbosity lives here.
# ============================================================================
from __future__ import annotations

import json
import os
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
DEFAULT_LOG_DIR = Path('.sego/work_logs')


def _default_log_path() -> Path:
    ts = int(time.time())
    return DEFAULT_LOG_DIR / f'session_{ts}.jsonl'


# ---------------------------------------------------------------------------
# Log entry types
# ---------------------------------------------------------------------------
@dataclass
class LogEntry:
    timestamp: str
    level: str          # phase | detail | error | routing | exec | turn | summary
    section: str         # context | commands | tools | routing | execution | persist | finish
    data: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


# ---------------------------------------------------------------------------
# WorkLog engine
# ---------------------------------------------------------------------------
class WorkLog:
    """Append-only JSONL work log that captures everything the user doesn't
    need to see inline.

    Usage::

        log = WorkLog()
        log.phase('context', elapsed_ms=12.3, detail='37 Python files found')
        log.routing('routing', matches=[...])
        log.execution('exec', command='bash', result='ok')
        log.flush()  # writes .jsonl
        print(log.summary_line)
    """

    def __init__(self, path: Path | None = None):
        self._path = path or _default_log_path()
        self._entries: list[LogEntry] = []
        self._started = time.time()
        self._errors: list[str] = []
        self._phase_count = 0
        self._completed_phases = 0
        self._failed_phases = 0

    # -- builders ------------------------------------------------------------

    def phase(self, section: str, *, status: str = 'completed',
              elapsed_ms: float = 0.0, message: str = '',
              detail: str = '', error: str = '') -> 'WorkLog':
        entry = LogEntry(
            timestamp=_iso_now(),
            level='phase',
            section=section,
            data={
                'status': status,
                'elapsed_ms': elapsed_ms,
                'message': message,
                'detail': detail,
                'error': error,
            },
        )
        self._entries.append(entry)
        self._phase_count += 1
        if status == 'completed':
            self._completed_phases += 1
        elif status == 'failed':
            self._failed_phases += 1
            self._errors.append(f'{section}: {error}')
        return self

    def routing(self, section: str, matches: list[dict[str, Any]]) -> 'WorkLog':
        entry = LogEntry(
            timestamp=_iso_now(),
            level='routing',
            section=section,
            data={'matches': matches, 'count': len(matches)},
        )
        self._entries.append(entry)
        return self

    def execution(self, section: str, *,
                  kind: str = '',
                  name: str = '',
                  message: str = '',
                  handled: bool = True) -> 'WorkLog':
        entry = LogEntry(
            timestamp=_iso_now(),
            level='exec',
            section=section,
            data={'kind': kind, 'name': name, 'message': message, 'handled': handled},
        )
        self._entries.append(entry)
        return self

    def turn(self, section: str, *,
             prompt: str = '',
             output: str = '',
             stop_reason: str = '',
             usage_input: int = 0,
             usage_output: int = 0) -> 'WorkLog':
        entry = LogEntry(
            timestamp=_iso_now(),
            level='turn',
            section=section,
            data={
                'prompt': prompt[:200],
                'output': output[:300],
                'stop_reason': stop_reason,
                'usage': {'input': usage_input, 'output': usage_output},
            },
        )
        self._entries.append(entry)
        return self

    def summary(self, section: str, **kwargs: Any) -> 'WorkLog':
        entry = LogEntry(
            timestamp=_iso_now(),
            level='summary',
            section=section,
            data=kwargs,
        )
        self._entries.append(entry)
        return self

    def detail(self, section: str, **kwargs: Any) -> 'WorkLog':
        entry = LogEntry(
            timestamp=_iso_now(),
            level='detail',
            section=section,
            data=kwargs,
        )
        self._entries.append(entry)
        return self

    def error(self, section: str, message: str, **kwargs: Any) -> 'WorkLog':
        entry = LogEntry(
            timestamp=_iso_now(),
            level='error',
            section=section,
            data={'message': message, **kwargs},
        )
        self._entries.append(entry)
        self._errors.append(f'{section}: {message}')
        return self

    # -- export / persistence -----------------------------------------------

    def flush(self) -> Path:
        """Write the entire log to a .jsonl file and return the path."""
        self._path.parent.mkdir(parents=True, exist_ok=True)
        if self._entries:
            with open(self._path, 'w', encoding='utf-8') as f:
                for entry in self._entries:
                    f.write(json.dumps(entry.to_dict(), ensure_ascii=False) + '\n')
        return self._path

    def to_dict(self) -> dict[str, Any]:
        return {
            'session_path': str(self._path),
            'started_at': _ts_to_iso(self._started),
            'elapsed_sec': round(time.time() - self._started, 3),
            'phase_count': self._phase_count,
            'completed_phases': self._completed_phases,
            'failed_phases': self._failed_phases,
            'entry_count': len(self._entries),
            'errors': self._errors,
        }

    @property
    def summary_line(self) -> str:
        d = self.to_dict()
        status = '❌' if d['failed_phases'] else '✅'
        base = '{} {}/{} phases ({}s) → {}'.format(
            status, d['completed_phases'], d['phase_count'],
            d['elapsed_sec'], d['session_path'])
        if d['errors']:
            base += ' | ERRORS: ' + ', '.join(d['errors'][:3])
        return base

    @property
    def has_errors(self) -> bool:
        return bool(self._errors)


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
def _iso_now() -> str:
    return time.strftime('%Y-%m-%dT%H:%M:%S', time.localtime())


def _ts_to_iso(ts: float) -> str:
    return time.strftime('%Y-%m-%dT%H:%M:%S', time.localtime(ts))
