# ============================================================================
# Sego Agent — Runtime (with progress-aware bootstrap)
# ============================================================================
from __future__ import annotations

import time
from dataclasses import dataclass

from .commands import PORTED_COMMANDS
from .context import PortContext, build_port_context, render_context
from .history import HistoryLog
from .models import PermissionDenial, PortingModule
from .query_engine import QueryEngineConfig, QueryEnginePort, TurnResult
from .setup import SetupReport, WorkspaceSetup, run_setup
from .system_init import build_system_init_message
from .tools import PORTED_TOOLS
from .execution_registry import build_execution_registry
from .progress_ui import ProgressUI, PhaseStatus
from .work_log import WorkLog


@dataclass(frozen=True)
class RoutedMatch:
    kind: str
    name: str
    source_hint: str
    score: int


@dataclass
class RuntimeSession:
    prompt: str
    context: PortContext
    setup: WorkspaceSetup
    setup_report: SetupReport
    system_init_message: str
    history: HistoryLog
    routed_matches: list[RoutedMatch]
    turn_result: TurnResult
    command_execution_messages: tuple[str, ...]
    tool_execution_messages: tuple[str, ...]
    stream_events: tuple[dict[str, object], ...]
    persisted_session_path: str

    def as_markdown(self) -> str:
        lines = [
            '# Runtime Session',
            '',
            f'Prompt: {self.prompt}',
            '',
            '## Context',
            render_context(self.context),
            '',
            '## Setup',
            f'- Python: {self.setup.python_version} ({self.setup.implementation})',
            f'- Platform: {self.setup.platform_name}',
            f'- Test command: {self.setup.test_command}',
            '',
            '## Startup Steps',
            *(f'- {step}' for step in self.setup.startup_steps()),
            '',
            '## System Init',
            self.system_init_message,
            '',
            '## Routed Matches',
        ]
        if self.routed_matches:
            lines.extend(
                f'- [{match.kind}] {match.name} ({match.score}) — {match.source_hint}'
                for match in self.routed_matches
            )
        else:
            lines.append('- none')
        lines.extend([
            '',
            '## Command Execution',
            *(self.command_execution_messages or ('none',)),
            '',
            '## Tool Execution',
            *(self.tool_execution_messages or ('none',)),
            '',
            '## Stream Events',
            *(f"- {event['type']}: {event}" for event in self.stream_events),
            '',
            '## Turn Result',
            self.turn_result.output,
            '',
            f'Persisted session path: {self.persisted_session_path}',
            '',
            self.history.as_markdown(),
        ])
        return '\n'.join(lines)


# ---------------------------------------------------------------------------
# Bootstrap result — returned by progress-aware bootstrap
# ---------------------------------------------------------------------------
@dataclass
class BootstrapResult:
    session: RuntimeSession
    work_log: WorkLog
    ui_summary: str  # single-line summary to show after progress bar


class PortRuntime:
    def route_prompt(self, prompt: str, limit: int = 5) -> list[RoutedMatch]:
        tokens = {token.lower() for token in prompt.replace('/', ' ').replace('-', ' ').split() if token}
        by_kind = {
            'command': self._collect_matches(tokens, PORTED_COMMANDS, 'command'),
            'tool': self._collect_matches(tokens, PORTED_TOOLS, 'tool'),
        }

        selected: list[RoutedMatch] = []
        for kind in ('command', 'tool'):
            if by_kind[kind]:
                selected.append(by_kind[kind].pop(0))

        leftovers = sorted(
            [match for matches in by_kind.values() for match in matches],
            key=lambda item: (-item.score, item.kind, item.name),
        )
        selected.extend(leftovers[: max(0, limit - len(selected))])
        return selected[:limit]

    # -----------------------------------------------------------------------
    # Progress-aware bootstrap (main entry point)
    # -----------------------------------------------------------------------
    def bootstrap_with_progress(self, prompt: str, limit: int = 5,
                                ui: ProgressUI | None = None) -> BootstrapResult:
        """Run bootstrap_session with live progress bars and a structured
        work log. Only compact progress output is printed; all details go to
        the WorkLog JSONL file."""
        work_log = WorkLog()
        t0 = time.perf_counter()

        # Build or reuse UI
        if ui is None:
            ui = ProgressUI('Sego Agent Bootstrap')
            ui.add_phase('context',  'Build workspace context')
            ui.add_phase('commands', 'Load command snapshots')
            ui.add_phase('tools',    'Load tool snapshots')
            ui.add_phase('routing',  'Route prompt to handlers')
            ui.add_phase('exec',     'Execute matched modules')
            ui.add_phase('persist',  'Persist session')
        ui.start()

        # --- Phase 1: context -------------------------------------------------
        _mark_phase(ui, 'context', PhaseStatus.RUNNING, 'scanning workspace...')
        context = build_port_context()
        elapsed = _elapsed(t0)
        ui.phase('context').elapsed_ms = elapsed
        ui.phase('context').status = PhaseStatus.COMPLETED
        ui.phase('context').message = f'{context.python_file_count} py files'
        ui.phase('context').detail = render_context(context)
        work_log.phase('context', elapsed_ms=elapsed,
                       detail=render_context(context),
                       message=f'{context.python_file_count} py files')
        _render_compact(ui, ui.phase('context'))

        # --- Phase 2: setup ---------------------------------------------------
        _mark_phase(ui, 'commands', PhaseStatus.RUNNING, 'setup + snapshots...')
        setup_report = run_setup(trusted=True)
        setup = setup_report.setup

        # commands snapshot
        cmd_names = [m.name for m in PORTED_COMMANDS]
        ui.phase('commands').elapsed_ms = _elapsed(t0)
        ui.phase('commands').status = PhaseStatus.COMPLETED
        ui.phase('commands').message = f'{len(PORTED_COMMANDS)} cmds'
        work_log.phase('commands', elapsed_ms=_elapsed(t0),
                       detail=f'{len(PORTED_COMMANDS)} commands: {", ".join(cmd_names[:20])}',
                       message=f'{len(PORTED_COMMANDS)} commands')

        # tools snapshot
        tool_names_list = [m.name for m in PORTED_TOOLS]
        ui.phase('tools').elapsed_ms = _elapsed(t0)
        ui.phase('tools').status = PhaseStatus.COMPLETED
        ui.phase('tools').message = f'{len(PORTED_TOOLS)} tools'
        work_log.phase('tools', elapsed_ms=_elapsed(t0),
                       detail=f'{len(PORTED_TOOLS)} tools: {", ".join(tool_names_list[:20])}',
                       message=f'{len(PORTED_TOOLS)} tools')

        # --- Phase 3: routing -------------------------------------------------
        _mark_phase(ui, 'routing', PhaseStatus.RUNNING, 'matching prompt...')
        matches = self.route_prompt(prompt, limit=limit)
        ui.phase('routing').elapsed_ms = _elapsed(t0)
        ui.phase('routing').status = PhaseStatus.COMPLETED
        ui.phase('routing').message = f'{len(matches)} matches'
        work_log.routing('routing', [
            {'kind': m.kind, 'name': m.name, 'score': m.score, 'source': m.source_hint}
            for m in matches
        ])

        # --- Phase 4: execution -----------------------------------------------
        _mark_phase(ui, 'exec', PhaseStatus.RUNNING, 'executing handlers...')
        registry = build_execution_registry()
        command_execs: list[str] = []
        tool_execs: list[str] = []
        denials: list[PermissionDenial] = []

        for match in matches:
            if match.kind == 'command':
                cmd = registry.command(match.name)
                if cmd:
                    result = cmd.execute(prompt)
                    command_execs.append(result)
                    work_log.execution('exec', kind='command', name=match.name,
                                       message=result, handled=True)
            elif match.kind == 'tool':
                tool = registry.tool(match.name)
                if tool:
                    result = tool.execute(prompt)
                    tool_execs.append(result)
                    work_log.execution('exec', kind='tool', name=match.name,
                                       message=result, handled=True)

        # deny destructive tools
        for match in matches:
            if match.kind == 'tool' and 'bash' in match.name.lower():
                d = PermissionDenial(tool_name=match.name,
                                     reason='destructive shell execution remains gated in the Python port')
                denials.append(d)
                work_log.execution('exec', kind='tool-gated', name=match.name,
                                   message=str(d.reason), handled=False)

        ui.phase('exec').elapsed_ms = _elapsed(t0)
        ui.phase('exec').status = PhaseStatus.COMPLETED
        ui.phase('exec').message = f'{len(command_execs)} cmd / {len(tool_execs)} tool'
        _render_compact(ui, ui.phase('exec'))

        # --- Phase 5: query engine & turn result -----------------------------
        _mark_phase(ui, 'persist', PhaseStatus.RUNNING, 'submitting turn...')
        engine = QueryEnginePort.from_workspace()
        stream_events = tuple(engine.stream_submit_message(
            prompt,
            matched_commands=tuple(m.name for m in matches if m.kind == 'command'),
            matched_tools=tuple(m.name for m in matches if m.kind == 'tool'),
            denied_tools=tuple(denials),
        ))
        turn_result = engine.submit_message(
            prompt,
            matched_commands=tuple(m.name for m in matches if m.kind == 'command'),
            matched_tools=tuple(m.name for m in matches if m.kind == 'tool'),
            denied_tools=tuple(denials),
        )
        persisted_session_path = engine.persist_session()

        work_log.turn('persist',
                      prompt=prompt,
                      output=turn_result.output,
                      stop_reason=turn_result.stop_reason,
                      usage_input=turn_result.usage.input_tokens,
                      usage_output=turn_result.usage.output_tokens)

        ui.phase('persist').elapsed_ms = _elapsed(t0)
        ui.phase('persist').status = PhaseStatus.COMPLETED
        ui.phase('persist').message = 'saved'
        _render_compact(ui, ui.phase('persist'))

        # --- finish -----------------------------------------------------------
        ui.finish(f'→ {persisted_session_path}')

        # Build history
        history = HistoryLog()
        history.add('context', f'python_files={context.python_file_count}, archive_available={context.archive_available}')
        history.add('registry', f'commands={len(PORTED_COMMANDS)}, tools={len(PORTED_TOOLS)}')
        history.add('routing', f'matches={len(matches)} for prompt={prompt!r}')
        history.add('execution', f'command_execs={len(command_execs)} tool_execs={len(tool_execs)}')
        history.add('turn', f'commands={len(turn_result.matched_commands)} tools={len(turn_result.matched_tools)} denials={len(turn_result.permission_denials)} stop={turn_result.stop_reason}')
        history.add('session_store', persisted_session_path)

        session = RuntimeSession(
            prompt=prompt,
            context=context,
            setup=setup,
            setup_report=setup_report,
            system_init_message=build_system_init_message(trusted=True),
            history=history,
            routed_matches=matches,
            turn_result=turn_result,
            command_execution_messages=tuple(command_execs),
            tool_execution_messages=tuple(tool_execs),
            stream_events=stream_events,
            persisted_session_path=persisted_session_path,
        )

        # Finalize work log
        total_ms = _elapsed(t0, as_float=False)
        work_log.summary('finish',
                         total_elapsed_ms=total_ms,
                         session_path=persisted_session_path,
                         phase_data=ui.extract_log_data())
        log_path = work_log.flush()

        ui_summary = (
            f'✅ Bootstrap complete | {len(matches)} matches | '
            f'work log: {log_path}'
        )

        return BootstrapResult(session=session, work_log=work_log, ui_summary=ui_summary)

    # -----------------------------------------------------------------------
    # Legacy bootstrap (kept for backward compat)
    # -----------------------------------------------------------------------
    def bootstrap_session(self, prompt: str, limit: int = 5) -> RuntimeSession:
        context = build_port_context()
        setup_report = run_setup(trusted=True)
        setup = setup_report.setup
        history = HistoryLog()
        engine = QueryEnginePort.from_workspace()
        history.add('context', f'python_files={context.python_file_count}, archive_available={context.archive_available}')
        history.add('registry', f'commands={len(PORTED_COMMANDS)}, tools={len(PORTED_TOOLS)}')
        matches = self.route_prompt(prompt, limit=limit)
        registry = build_execution_registry()
        command_execs = tuple(registry.command(match.name).execute(prompt) for match in matches if match.kind == 'command' and registry.command(match.name))
        tool_execs = tuple(registry.tool(match.name).execute(prompt) for match in matches if match.kind == 'tool' and registry.tool(match.name))
        denials = tuple(self._infer_permission_denials(matches))
        stream_events = tuple(engine.stream_submit_message(
            prompt,
            matched_commands=tuple(match.name for match in matches if match.kind == 'command'),
            matched_tools=tuple(match.name for match in matches if match.kind == 'tool'),
            denied_tools=denials,
        ))
        turn_result = engine.submit_message(
            prompt,
            matched_commands=tuple(match.name for match in matches if match.kind == 'command'),
            matched_tools=tuple(match.name for match in matches if match.kind == 'tool'),
            denied_tools=denials,
        )
        persisted_session_path = engine.persist_session()
        history.add('routing', f'matches={len(matches)} for prompt={prompt!r}')
        history.add('execution', f'command_execs={len(command_execs)} tool_execs={len(tool_execs)}')
        history.add('turn', f'commands={len(turn_result.matched_commands)} tools={len(turn_result.matched_tools)} denials={len(turn_result.permission_denials)} stop={turn_result.stop_reason}')
        history.add('session_store', persisted_session_path)
        return RuntimeSession(
            prompt=prompt,
            context=context,
            setup=setup,
            setup_report=setup_report,
            system_init_message=build_system_init_message(trusted=True),
            history=history,
            routed_matches=matches,
            turn_result=turn_result,
            command_execution_messages=command_execs,
            tool_execution_messages=tool_execs,
            stream_events=stream_events,
            persisted_session_path=persisted_session_path,
        )

    def run_turn_loop(self, prompt: str, limit: int = 5, max_turns: int = 3, structured_output: bool = False) -> list[TurnResult]:
        engine = QueryEnginePort.from_workspace()
        engine.config = QueryEngineConfig(max_turns=max_turns, structured_output=structured_output)
        matches = self.route_prompt(prompt, limit=limit)
        command_names = tuple(match.name for match in matches if match.kind == 'command')
        tool_names = tuple(match.name for match in matches if match.kind == 'tool')
        results: list[TurnResult] = []
        for turn in range(max_turns):
            turn_prompt = prompt if turn == 0 else f'{prompt} [turn {turn + 1}]'
            result = engine.submit_message(turn_prompt, command_names, tool_names, ())
            results.append(result)
            if result.stop_reason != 'completed':
                break
        return results

    def _infer_permission_denials(self, matches: list[RoutedMatch]) -> list[PermissionDenial]:
        denials: list[PermissionDenial] = []
        for match in matches:
            if match.kind == 'tool' and 'bash' in match.name.lower():
                denials.append(PermissionDenial(tool_name=match.name, reason='destructive shell execution remains gated in the Python port'))
        return denials

    def _collect_matches(self, tokens: set[str], modules: tuple[PortingModule, ...], kind: str) -> list[RoutedMatch]:
        matches: list[RoutedMatch] = []
        for module in modules:
            score = self._score(tokens, module)
            if score > 0:
                matches.append(RoutedMatch(kind=kind, name=module.name, source_hint=module.source_hint, score=score))
        matches.sort(key=lambda item: (-item.score, item.name))
        return matches

    @staticmethod
    def _score(tokens: set[str], module: PortingModule) -> int:
        haystacks = [module.name.lower(), module.source_hint.lower(), module.responsibility.lower()]
        score = 0
        for token in tokens:
            if any(token in haystack for haystack in haystacks):
                score += 1
        return score


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
def _mark_phase(ui: ProgressUI, key: str, status: PhaseStatus, message: str) -> None:
    p = ui.phase(key)
    p.status = status
    p.message = message
    _render_compact(ui, p)


def _render_compact(ui: ProgressUI, phase: 'Phase | None' = None) -> None:
    # Only print if stdout is a real terminal (not piped)
    import sys as _sys
    if _sys.stdout.isatty():
        from .progress_ui import Phase
        ui._render_compact(phase)


def _elapsed(t0: float, as_float: bool = True):
    """Return elapsed milliseconds since t0 (perf_counter)."""
    ms = (time.perf_counter() - t0) * 1000
    return ms if as_float else int(ms)
