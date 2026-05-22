# ============================================================================
# Sego Agent — Progress UI
# Compact phase-based progress bars + spinner.
# Only minimal status lines are printed to stdout.
# All detailed execution info is routed to work_log.py.
# ============================================================================
from __future__ import annotations

import shutil
import sys
import time
from dataclasses import dataclass, field
from enum import Enum
from typing import Callable

# ---------------------------------------------------------------------------
# ANSI terminal helpers
# ---------------------------------------------------------------------------
if sys.platform == 'win32':
    try:
        import ctypes
        _kernel32 = ctypes.windll.kernel32
        _kernel32.SetConsoleMode(_kernel32.GetStdHandle(-11), 7)  # ENABLE_VIRTUAL_TERMINAL_PROCESSING
    except Exception:
        pass


class _Style:
    RESET = '\033[0m'
    BOLD = '\033[1m'
    DIM = '\033[2m'

    class FG:
        GREEN = '\033[32m'
        YELLOW = '\033[33m'
        BLUE = '\033[34m'
        CYAN = '\033[36m'
        RED = '\033[31m'
        WHITE = '\033[37m'

    class BG:
        GREEN = '\033[42m'
        RED = '\033[41m'


TERM_WIDTH = shutil.get_terminal_size((80, 24)).columns


# ---------------------------------------------------------------------------
# Phase status
# ---------------------------------------------------------------------------
class PhaseStatus(Enum):
    PENDING = 'pending'
    RUNNING = 'running'
    COMPLETED = 'completed'
    FAILED = 'failed'
    SKIPPED = 'skipped'


@dataclass
class Phase:
    """A single named phase in the execution pipeline."""
    name: str
    status: PhaseStatus = PhaseStatus.PENDING
    message: str = ''
    started_at: float | None = None
    finished_at: float | None = None
    elapsed_ms: float = 0.0
    detail: str = ''           # routed to work_log, not displayed inline
    error: str = ''            # routed to work_log


# ---------------------------------------------------------------------------
# Spinner frames
# ---------------------------------------------------------------------------
_SPINNER = ('⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏')
_STATUS_ICON: dict[PhaseStatus, str] = {
    PhaseStatus.PENDING:   ' ',
    PhaseStatus.RUNNING:   _Style.FG.CYAN + '◷' + _Style.RESET,
    PhaseStatus.COMPLETED: _Style.FG.GREEN + '✔' + _Style.RESET,
    PhaseStatus.FAILED:    _Style.FG.RED + '✘' + _Style.RESET,
    PhaseStatus.SKIPPED:   _Style.DIM + '○' + _Style.RESET,
}


# ---------------------------------------------------------------------------
# Bar drawing
# ---------------------------------------------------------------------------
def _bar(completed: int, total: int, width: int = 20) -> str:
    ratio = min(completed / max(total, 1), 1.0)
    filled = int(ratio * width)
    pct = int(ratio * 100)

    if ratio >= 1.0:
        bar_color = _Style.FG.GREEN
        pct_color = _Style.FG.GREEN
    elif ratio >= 0.5:
        bar_color = _Style.FG.YELLOW
        pct_color = _Style.FG.YELLOW
    else:
        bar_color = _Style.FG.CYAN
        pct_color = _Style.FG.WHITE

    bar_str = bar_color + '█' * filled + _Style.DIM + '░' * (width - filled) + _Style.RESET
    return f'{bar_color}[{bar_str}{bar_color}] {pct_color}{pct:3d}%{_Style.RESET}'


def _elapsed_str(ms: float) -> str:
    if ms < 1000:
        return f'{ms:5.0f}ms'
    return f'{ms/1000:5.1f}s'


# ---------------------------------------------------------------------------
# Progress UI engine
# ---------------------------------------------------------------------------
@dataclass
class ProgressUI:
    """Manages a set of phases and renders compact progress output.

    Usage::

        ui = ProgressUI('Sego Agent Bootstrap')
        ui.add_phase('context', 'Build workspace context')
        ui.add_phase('commands', 'Load command snapshots')
        ui.add_phase('tools', 'Load tool snapshots')
        ui.add_phase('routing', 'Route prompt to handlers')
        ui.add_phase('exec', 'Execute matched modules')
        ui.add_phase('persist', 'Persist session')

        ui.start()
        ui.phase('context').complete(detail='37 Python files found')
        ui.phase('commands').complete(detail='64 commands mirrored')
        # … etc
        ui.finish()
    """
    title: str
    phases: list[Phase] = field(default_factory=list)
    _phase_index: dict[str, int] = field(default_factory=dict)
    _spinner_idx: int = 0
    _started: bool = False
    _finished: bool = False
    _last_line_count: int = 0
    _on_complete_callback: Callable[[ProgressUI], None] | None = None
    _compact: bool = True  # single-line mode when True

    def add_phase(self, key: str, name: str) -> 'ProgressUI':
        self._phase_index[key] = len(self.phases)
        self.phases.append(Phase(name=name))
        return self

    def phase(self, key: str) -> Phase:
        return self.phases[self._phase_index[key]]

    # -- lifecycle ----------------------------------------------------------

    def start(self) -> None:
        self._started = True
        self._print_header()

    def finish(self, final_message: str = '') -> None:
        self._finished = True
        if not self._compact:
            self._render_full()
        self._print_footer(final_message)
        if self._on_complete_callback:
            self._on_complete_callback(self)

    def set_on_complete(self, callback: Callable[[ProgressUI], None]) -> None:
        self._on_complete_callback = callback

    # -- render -------------------------------------------------------------

    def _print_header(self) -> None:
        bar = '━' * min(len(self.title) + 4, TERM_WIDTH - 2)
        print(f'\n{_Style.BOLD}{_Style.FG.CYAN}{bar}{_Style.RESET}')
        print(f'{_Style.BOLD}  {self.title}{_Style.RESET}')
        print(f'{_Style.FG.CYAN}{bar}{_Style.RESET}\n')

    def _print_footer(self, final_message: str = '') -> None:
        bar = '━' * min(36, TERM_WIDTH - 2)
        print(f'\n{_Style.FG.CYAN}{bar}{_Style.RESET}')
        completed = sum(1 for p in self.phases if p.status == PhaseStatus.COMPLETED)
        failed = sum(1 for p in self.phases if p.status == PhaseStatus.FAILED)
        total = len(self.phases)
        total_ms = sum(p.elapsed_ms for p in self.phases)

        status_color = _Style.FG.RED if failed else _Style.FG.GREEN
        print(f'  {status_color}{completed}/{total} phases completed '
              f'({_elapsed_str(total_ms)}){_Style.RESET}')
        if failed:
            failed_names = [p.name for p in self.phases if p.status == PhaseStatus.FAILED]
            print(f'  {_Style.FG.RED}Failed: {", ".join(failed_names)}{_Style.RESET}')
        if final_message:
            print(f'  {final_message}')
        print(f'{_Style.FG.CYAN}{bar}{_Style.RESET}\n')

    def _render_full(self) -> None:
        """Render all phases with individual status lines."""
        for p in self.phases:
            icon = _STATUS_ICON[p.status]
            msg = f' {p.message}' if p.message else ''
            elapsed = f' {_elapsed_str(p.elapsed_ms)}' if p.elapsed_ms else ''
            print(f'  {icon} {p.name}{msg}{elapsed}')

    def _render_compact(self, current_phase: Phase | None = None) -> None:
        """Render a single compact progress bar line."""
        completed = sum(1 for p in self.phases if p.status == PhaseStatus.COMPLETED)
        failed = sum(1 for p in self.phases if p.status == PhaseStatus.FAILED)
        total = len(self.phases)

        spinner = _SPINNER[self._spinner_idx % len(_SPINNER)]
        self._spinner_idx += 1

        if failed:
            bar_color = _Style.FG.RED
        else:
            bar_color = _Style.FG.CYAN

        bar = _bar(completed + failed, total)
        running_name = current_phase.name if current_phase else ''
        tail = ''
        if current_phase and current_phase.message:
            tail = f' — {_Style.DIM}{current_phase.message}{_Style.RESET}'

        # Terminal width aware truncation
        prefix = f' {bar_color}{spinner}{_Style.RESET} {bar} '
        available = TERM_WIDTH - len(prefix) - len(tail) - 2  # escape code aware — rough
        name = running_name[:max(available, 8)] if running_name else '...'
        line = f'{prefix}{_Style.BOLD}{name}{_Style.RESET}{tail}'
        # Pad to clear previous line
        pad = max(TERM_WIDTH - _visible_len(line), 0)
        print(f'\r{line}{" " * pad}', end='', flush=True)

    # -- dynamic display mode -----------------------------------------------

    def set_compact(self, value: bool) -> None:
        self._compact = value

    # -- extract work-log data ----------------------------------------------

    def extract_log_data(self) -> list[dict[str, object]]:
        """Return phase details suitable for work_log persistence."""
        return [
            {
                'phase': p.name,
                'status': p.status.value,
                'elapsed_ms': p.elapsed_ms,
                'message': p.message,
                'detail': p.detail,
                'error': p.error,
            }
            for p in self.phases
        ]


# ---------------------------------------------------------------------------
# Utility
# ---------------------------------------------------------------------------
def _visible_len(s: str) -> int:
    """Approximate visible length of an ANSI-escaped string."""
    import re
    return len(re.sub(r'\x1b\[[0-9;]*m', '', s))
