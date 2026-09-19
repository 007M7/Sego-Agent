# Architecture diagrams

Six diagrams covering the views the architecture baseline asks for: system context,
container, call chain, state transition, data ownership, and trust boundary.

They exist because the baseline document had no drawings, and a table is not one.
Each figure is a **vector source** (`.tex`, TikZ) with the rendered `.svg` beside it,
so the drawing can be corrected rather than redrawn, and the diff shows what changed.

| # | view | source | rendered | what it answers |
|---|---|---|---|---|
| 6 | system context | [`fig6-system-context.tex`](../assets/figures/fig6-system-context.tex) | [svg](../assets/figures/fig6-system-context.svg) | who Sego talks to, and what crosses each boundary |
| 7 | container | [`fig7-container.tex`](../assets/figures/fig7-container.tex) | [svg](../assets/figures/fig7-container.svg) | what runs, what it keeps on disk, what is outside the process |
| 8 | call chain | [`fig8-call-chain.tex`](../assets/figures/fig8-call-chain.tex) | [svg](../assets/figures/fig8-call-chain.svg) | one review, function by function, and where each identity is decided |
| 9 | state transition | [`fig9-state-machines.tex`](../assets/figures/fig9-state-machines.tex) | [svg](../assets/figures/fig9-state-machines.svg) | the three machines that are routinely read as one |
| 10 | data ownership | [`fig10-data-ownership.tex`](../assets/figures/fig10-data-ownership.tex) | [svg](../assets/figures/fig10-data-ownership.svg) | who may write each fact, and who may only read it |
| 11 | trust boundary | [`fig11-trust-boundary.tex`](../assets/figures/fig11-trust-boundary.tex) | [svg](../assets/figures/fig11-trust-boundary.svg) | what Sego observed, what it must treat as a claim |

Figures 1–5 live beside these and cover motivation, core architecture, the review
pipeline, artifact lifecycle and integration topology; the README renders them.

## What each one is grounded in

A diagram that cannot be checked is a drawing, not a claim. Every figure names the
values it draws, and each is taken from the code or the schema rather than from
memory.

- **Figure 6** draws `ReviewScope` and the sidecar envelope as the two ways in, the
  resolved provider as the only egress on the review path, and `WebFetch` as the
  second one. The claim the caption makes — that Sego writes no governance fact —
  is the same one the data-ownership figure draws as a rule.
- **Figure 7** lists the workspace members as they are: `rusty-claude-cli`,
  `runtime`, `tools`, `api`, `commands`, `plugins`, `compat-harness`, `telemetry`.
  The persisted stores are the three paths the code actually writes.
- **Figure 8** names real callables: `sidecar::execute_review`,
  `collect_review_target`, `resolve_strict_routing`, `run_review_turn`,
  `evaluate_evidence_gate`, `persist_review_artifact_observed`. The artifact id
  format, the `create_new` refusal and the `artifact_id_conflict` outcome are
  current behaviour, not aspiration.
- **Figure 9** serialises the same labels the code does: `ReviewParseStatus`
  (`structured`, `fallback_raw_text`, `parse_attempted_but_failed`, plus `no_diff`
  on the empty-scope shortcut), `ReviewFindingStatus` (six labels, three of which
  require a note), and `ReviewCardConfidence` (`Green` / `Yellow` / `Red`).
- **Figure 10** draws the ownership rule the cross-domain protocol states in its
  first clause: one writer per fact, and cross-domain references that carry no
  write authority.
- **Figure 11** draws the trust model as `SEG-ADR-005` fixed it: model-assisted,
  with `identity_evidence` staying `self_reported_resolution` and never becoming a
  provider attestation, and with the egress fields observed rather than configured.

## Regenerating

Requires a LaTeX installation with TikZ, plus `dvisvgm`. No other toolchain.

```bash
cd assets/figures
pdflatex -interaction=nonstopmode -halt-on-error fig6-system-context.tex
dvisvgm --pdf --no-fonts --exact-bbox -o fig6-system-context.svg fig6-system-context.pdf
```

`--no-fonts` turns glyphs into paths, which is why the SVGs are large — the six
together add about 5.5 MB. That matches figures 1–5, which are built the same way,
and it keeps them rendering identically without the TeX fonts installed. PNG copies
are **not** committed: nothing referenced the PNGs for figures 1–5, and `AGENTS.md`
asks contributors not to add large binary assets. The SVG is the render used by the
docs; the `.tex` is what to edit.

## Layout constraints worth keeping

Three defects were found by rendering and looking, and each is worth not
reintroducing:

- **A fixed `text width` per state box.** Letting a node size to its text put
  `parse_attempted_but_failed` on top of its neighbour in figure 9.
- **Arrows that meet the boundary, not inner nodes.** Routing the external arrows
  in figure 7 from inside the container made them cross the whole figure.
- **No names that collide with TikZ keys.** `out`, `step` and `key` are taken;
  `pgfkeys` rejects the style with "requires a value" rather than failing quietly.
  Figures 7 and 8 use `ext`, `bx` and `gatebox` for that reason.
