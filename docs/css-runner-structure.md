# CSS runner module map

The native fitting editor starts at `src/atelier/css_runner/mod.rs`. This small
facade declares the implementation modules and re-exports the existing public
API. Callers can continue to use `atelier::css_runner::CssRunnerState`,
`build_runner_ui`, `create_runner_app`, the public geometry helpers/action flags,
and `native_board::*`.

## Where changes belong

| Area | Files under `src/atelier/css_runner/` | Responsibility |
| --- | --- | --- |
| Startup | `app.rs` | App construction, viewport hookup, window/worker startup |
| State | `state.rs` | Shared editor state, selection helpers and simulation control methods |
| Actions | `actions.rs` | Action flags, queued UI operations and the frame update loop |
| Documents | `documents.rs` | Save and export methods |
| Imports | `imports.rs`, `ui/import_dialog.rs` | Reviewed DXF/SVG/avatar parsing, preview, confirmation and transactional installation |
| Editing | `editing.rs` | Authoring transactions, history, curve/corner edits and draft commits |
| Board coordinates | `board/geometry.rs`, `board/types.rs` | Layout bounds, coordinate conversion, geometry helpers and transient types |
| Board selection | `board/hit_testing.rs`, `board/selection.rs` | Picking, linked/sewn selection and selection cleanup |
| Board tools | `board/corners.rs`, `board/interaction.rs` | Fillet limits/previews and tool/draft initialization |
| Board display | `board/rendering.rs`, `board/native.rs` | Native vector scenes and typed drawing primitives |
| Input | `input/{mod,keyboard,buttons,motion,scroll}.rs` | Event routing and focused handlers |
| Shell | `ui/shell.rs`, `ui/viewports.rs` | Workspace composition, viewport hosts and responsive CAD toolbar |
| Chrome | `ui/top_bar.rs`, `ui/tool_rails.rs`, `ui/status_bar.rs`, `ui/command_palette.rs`, `ui/settings.rs` | Transport, tools, status, commands and session settings |
| Browsers | `ui/asset_browser.rs`, `ui/inspector/` | Assets/piece cards, category tabs and property accordions |
| Regression tests | `tests/{board,shell,simulation,imports}.rs`, `tests/support.rs` | Existing board/editing, shell/handler, camera and import regressions with shared fixtures |

Each implementation file imports its dependencies explicitly. Private helper
visibility is restricted to the runner; the facade preserves the original
external entry points. The state owns the same per-instance action queues, worker
mailbox, document/history and transient selection/drag data as before. Saving and
export remain `CssRunnerState` methods implemented in `documents.rs`.

The input router retains focus/resize handling and delegates keyboard, button,
motion and scroll events to their original match arms. Button handling still
owns the existing tool gestures and capture/release behavior. The board display
continues to use the cached grid and uncached drawing leaves described in
[cssimpler integration](cssimpler-integration.md).

## Scope of this refactor

This is the module-split stage. Existing UI layout, text, document formats,
transactions, simulation behavior and rendering commands are preserved. The
central state struct stays together. Widget extraction and further subdivision
of longer tool/property functions are follow-up work, starting from the boundaries
above.

The existing regression tests are moved by responsibility with their assertions
retained. Validation uses the CPU and desktop suites, all-target Clippy and
repository formatting. Public-path compatibility is also exercised by the native
board integration tests. Native window/GPU acceptance is separate from these
headless checks.

Validation for this split: the full desktop suite passed 771 tests (27 opt-in
checks ignored), and the CPU-only suite passed 658 (one ignored). Strict
all-target desktop and CPU Clippy and repository formatting passed. A move audit
also compared 83 function/test bodies, 15 state methods and seven event match
arms with the original runner; their bodies are unchanged apart from qualified
module paths and formatting.

The [refactor report](reports/css-runner-refactor.md) records the changes,
decision rationale, validation evidence and deferred widget work.
