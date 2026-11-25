# Formal verification

Much of waft's decision logic is machine-checked with
[Verus](https://github.com/verus-lang/verus), a verifier for Rust. This record
says which decisions are verified, what each proof establishes, what the proofs
assume, and which parts of waft stay outside the verified boundary.
[ASSURANCE.md](ASSURANCE.md) covers tested behavior and operational limits;
this file covers proofs.

## Summary

| | |
| --- | --- |
| Verified crate | `waft-verified` (`verified/`): 933 items verified, 0 errors, with Verus's `--no-cheating` check on the crate |
| Trusted items in that crate | None: no `assume`, `admit`, `external_body`, `assume_specification`, `uninterp` or `verus::trusted`. The proofs still rely on 17 traits production implements, one unverified `vstd` function and `vstd`'s specifications of the standard library; see [Trust boundary](#trust-boundary). |
| Kernel size | 4,984 executable lines, with 9,317 specification and 9,022 proof lines (Verus `line_count`) |
| Production use | Ten areas of `src/` hand their main decisions to the kernels: paths, context and CLI policy, configuration, validation and reports, the Git backend, the rule engine, planning, execution and the index lock, file checks, and publication. What stays in production is listed under [Outside the verified boundary](#outside-the-verified-boundary). |
| Production code | 6,757 non-test, non-comment, non-blank lines in `src/` on `main` before this work, 7,290 now. The growth is glue: conversions between production and kernel types, the trait implementations, and the drivers that perform the effects the kernels request. |
| Behavior | Identical to `main` across 2,321 end-to-end CLI invocation pairs; one reachable panic is now an existing error ([details](#behavior-and-cost)) |
| Cost | Release binary +1.7%; nine more crates to compile (`waft-verified`, `vstd` and seven Verus support crates); the Verus check takes about half a minute |

## What "verified" means here

The kernels are pure decision functions and step machines in `waft-verified`.
Production converts its own values into kernel inputs, calls a kernel, and acts
on the answer. Verus checks each kernel's executable code against its
specification. An ordinary `cargo build` erases specifications and proofs, so
the production binary is built from exactly the executable source Verus
checked. Only `use` imports of specification items differ between the two
builds. The production compiler is Rust 1.90, while Verus reads the code with
its own Rust 1.100.0 front end.

Terms used throughout:

- **Oracle:** a trait a kernel calls to ask about the world (a file's
  identity, a matcher's verdict, an environment variable), implemented by
  production. Verus takes the implementation's answers as given.
- **Ghost log or trace:** a specification-only record, erased from the
  production build, of the calls or effects a kernel has requested, in order.
  Postconditions about the log state exactly which effects happen, and in what
  order.
- **Model:** a hand-written specification of something outside the crate,
  such as the operating system, Git, or a library's matcher, that lemmas reason
  about.
- **Worktree family:** a repository's main worktree and its linked
  worktrees.
- **`git`, `claude-2026-04`, `wt-0.39`:** the three `.worktreeinclude`
  semantics waft offers (see
  [docs/worktreeinclude-compatibility.md](docs/worktreeinclude-compatibility.md)).

The guarantees below come in three kinds, and each section says which kind it
relies on:

- **Postconditions of the functions production calls.** These hold whenever
  the function's `requires` clause holds; a `requires` clause is not checked
  at run time when unverified code calls the kernel. Two functions production
  calls have one, `index_lock::acquire` and `execute::failure_summary`.
  Production establishes both at its call sites, except that the public
  library function `executor::report_has_failures` accepts a hand-built
  `CopyReport` that can break the second (see the executor section). Review,
  not Verus, checked the call sites.
- **Type invariants and typestate.** The publication step machines, the
  streaming comparison and the fingerprinter are values that exist only after
  the steps their type records were fed answers, in order. Rust privacy stops
  production from building them directly. Two publication steps record a
  destination proof without receiving it. The planner's step kernels take
  plain observation structs that production fills in, so there the order rests
  on production (see those sections). Nothing proves that production ran the
  system call whose answer it fed in.
- **Lemmas.** Some lemmas are about a kernel's own specification, so they
  reach production through the postconditions. Others are about hand-written
  models:
  - the index-lock signal model;
  - the publication filesystem model;
  - the reference model of `ignore` and Git's per-directory excludes;
  - a model of Git's `-z` output writers;
  - the index-cache slot model;
  - a model of `RealFs::source_kind`.

  A few postconditions are stated over hand-written models of `std`:
  `std_components` for Unix `Path::components`, and `line_at` and `trim` for
  `str::lines` and `str::trim`. Nothing mechanically ties these models to the
  operating system, `std`, `ignore` or Git. The sections say where each model
  is used and which are pinned by tests.

## Running the proofs

```sh
just check-verus          # or: bash scripts/verus-verify.sh
```

The script:

- downloads the pinned Verus release (0.2026.11.22.1ce6a59) for Linux x86-64 and
  checks its SHA-256. Elsewhere, set `VERUS_HOME` to an unpacked release of
  the same version; the script then skips the download.
- installs the Rust toolchain that release needs (1.100.0) if it
  is missing.
- runs `cargo verus verify -p waft-verified`, passing `--no-cheating` to
  `waft-verified` only. `vstd`, which contains the trusted specifications of
  `std`, is verified without it.
- re-verifies on every run, and fails unless Verus reports a clean result.

`--no-cheating` makes Verus reject `assume`, `admit`, `external_body`,
`assume_specification`, `uninterp` and trusted items in the checked crate. CI
runs the script in the `formal verification (Verus)` job, which `core checks`
requires.

The production build does not need Verus: `waft-verified` compiles with the
repository's Rust 1.90 toolchain and the `vstd` crate from crates.io.
`cargo verus verify` compiles that same `vstd` crate with Verus. Its version is
pinned to the one the Verus release expects, so Dependabot ignores it. Update
the script's version, checksum and toolchain together with `vstd` in
`verified/Cargo.toml`.

While developing a kernel, verify one module at a time with
`cargo verus focus -p waft-verified -- --verify-module <module>`, with the
Verus directory on `PATH`. `--no-cheating` cannot be combined with
`--verify-module`, so finish with the full script.

## What is verified

Each row is a part of `src/` that hands its main decisions to verified kernels.
The line counts are specification, proof and executable lines in the area's
kernels, from Verus's `line_count` tool. `lint.rs` was counted with the
`build/*` text in two doc comments altered, because the tool reads it as the
start of a block comment. The tool marks no line in any kernel as trusted. The
contracts of the traits production implements are counted as specification
and executable lines, and the [Trust boundary](#trust-boundary) lists them.

| Area | Production | Decisions now made by verified code | Spec / proof / exec lines |
| --- | --- | --- | --- |
| [Repository-relative paths](#repository-relative-paths) | `src/path.rs` | Lexical `.` and `..` normalization; whether a path is outside, at, or inside the repository root; forbidden characters; Windows verbatim prefixes | 326 / 498 / 187 |
| [Repository context, CLI policy and walk boundaries](#repository-context-cli-policy-and-walk-boundaries) | `src/context.rs`, `src/cli.rs`, `src/walk.rs` | Main-worktree selection; source and destination resolution and which error is reported; user-config source precedence; worktree path mapping; repository boundaries during walks; `.git` file parsing | 566 / 565 / 371 |
| [Configuration policy](#configuration-policy) | `src/config.rs` | Setting names and values; layer precedence and profiles; TOML validation and its first error; `WAFT_*` parsing; project-config discovery boundary; user-config location | 1,046 / 1,400 / 578 |
| [Validation, lint and list/info reports](#validation-lint-and-listinfo-reports) | `src/validate.rs`, `src/subcommands/` | Validation gate and severities; rule-file symlink handling; lint findings; `list` and `info` action labels | 666 / 462 / 440 |
| [Git backend](#git-backend) | `src/git.rs` | Tracked-path lookup; parsing of Git output; backend and case-folding policy; index cache step; rule-file existence | 1,310 / 1,493 / 659 |
| [Rule engine](#rule-engine) | `src/worktreeinclude.rs`, `src/worktreeinclude_engine.rs`, `src/policy_filter.rs`, `src/subcommands/mod.rs` | Rule-file precedence and verdicts; rule-line grammar; exclusion verdicts; which rules apply; empty-selection diagnosis | 922 / 883 / 366 |
| [Planner](#planner) | `src/planner.rs`, `src/eligibility_groups.rs`, `src/model.rs` | Per-file plan decisions; destination classification; directory promotion; manifest ordering and deduplication; dry-run accounting and exit status | 929 / 1,006 / 646 |
| [Executor and Git index lock](#executor-and-git-index-lock) | `src/executor.rs` | Plan loop and counters; output and exit-status policy; index-lock retries; lock cleanup when a signal arrives | 1,368 / 968 / 423 |
| [File checks: fingerprints, path components, comparison and stability](#file-checks-fingerprints-path-components-comparison-and-stability) | `src/fs.rs` | Content fingerprints; safe path walks; file comparison; stability checks; errno classification; retry and directory-creation steps | 664 / 693 / 311 (+10) |
| [Publication protocols](#publication-protocols) | `src/fs.rs` (Unix) | No-clobber publication; which names cleanup may remove; exchange and displacement replacement with undo; permission repair; step order | 1,520 / 1,054 / 1,003 (+42) |
| **Total** | | | **9,317 / 9,022 / 4,984 (+52)** |

A "(+n)" counts lines that hold both proof and executable code.

The sections below cover each area in the same layout:

- what production delegates;
- what the proofs establish, and what each guarantee is attached to;
- what is assumed;
- what is still unverified.

## Areas

### Repository-relative paths

**Production code:** `src/path.rs`. **Kernels:**
`waft_verified::path::{model, normalize, containment, text}`.

`RepoRelPath::normalize` turns command-line paths, walker entries,
`.worktreeinclude` negation targets and gitlink checks into
repository-relative names. The kernels decide which components survive
lexical `.`/`..` normalization, whether the result is outside the root, the
root itself, or strictly inside it with only names left, and whether a path
string contains NUL, CR or LF. On Windows they also decide how a UTF-8
path's `\\?\` prefix is removed.

**Verified decisions**
- Surviving components of the input and of the root:
  `normalize::logical_normalize`, via the private `logical_normalize` wrapper,
  in `RepoRelPath::normalize`.
- Outside, the root itself, or strictly inside, and where the names start:
  `containment::classify_containment` in `RepoRelPath::normalize`, replacing
  `Path::strip_prefix`, the empty check and the names-only loop. Its
  component-prefix test, `containment::component_prefix`, is shared with the
  worktree-mapping kernel (see the context section).
- NUL, CR or LF: `text::has_forbidden_text_char` in `validate_text_path`,
  which checks every `normalize` result and every `from_git_bytes` path.
- Windows only, how a UTF-8 `\\?\UNC\` or `\\?\` path is rewritten:
  `text::classify_verbatim_prefix` in `without_verbatim_prefix_text`, which
  the Windows `without_windows_verbatim_prefix` calls; `normalize`,
  `normalize_repo_path` in `src/git.rs` and
  `Cli::map_worktree_path_to_source` in `src/cli.rs` call that function. On
  other platforms it is the identity and the kernel runs only in tests.

**What the proofs establish**
- `logical_normalize` returns strictly increasing, in-bounds indices that
  select exactly `spec_normalize`, a stack: prefixes and names are pushed,
  the root discards everything but prefixes and is pushed, `.` is dropped,
  and `..` pops a name on top of the stack or is otherwise pushed. The kernel
  mirrors the existing behavior, including keeping `..` directly after the
  root (`/../x` stays `/../x`). *Postcondition.*
- `classify_containment` returns `spec_containment` of the two lists' views:
  `Outside` unless the root is a component-wise prefix, `RepoRoot` if equal,
  `Inside(root length)` if only `Normal` components remain, `Outside`
  otherwise. `component_prefix(root, abs)` returns exactly whether the root's
  views are a component-wise prefix of the path's. *Postconditions.*
- The result is in normal form (no `.`, the root only after prefixes, no `..`
  after a name); fixed points are exactly normal forms; normalizing is
  idempotent. `Inside` means the path list is the root list followed by a
  non-empty list of `Normal` components; `RepoRoot` means the lists are
  equal. Over raw lists (`spec_repo_relative`), an `Inside` decision splits
  the normalized path into the normalized root followed by the result, and
  non-empty names appended to any root classify back to exactly those names
  (`theorem_round_trip`, the list-level counterpart of
  `normalize(p.to_path(root), root) == Ok(p)`). *Lemmas about the kernel's
  specification.*
- The one behavior change. The non-name-remains branch, formerly
  `unreachable!`, can be taken only when the normalized root has no name
  (`theorem_escape_needs_nameless_root`), and every such root takes it for
  the list of `root/..` (`theorem_nameless_root_escapes`). Such roots
  normalize to `""`, a bare root or prefix (`/`, `C:`, `C:\`), or one of
  these followed only by `..`s: `.`, `/..` and `/repo/../..` are examples.
  When the branch is taken, the first remaining component is the non-name
  (`theorem_escape_is_leading`), so the old loop panicked before converting
  any name. `normalize` now returns the existing "outside the repository
  root" error, for example for `../x` under `/`. *Lemmas about the kernel's
  specification. The first assumes the raw path list has a prefix only
  first; the third assumes the normalized path is names-last, which
  `theorem_names_last` derives from that assumption.*
- `has_forbidden_text_char(s)` returns whether `s@`, vstd's character view
  of the string, contains `'\n'`, `'\r'` or `'\0'`; it scans bytes, which
  `lemma_forbidden_byte_iff_char` justifies for UTF-8. *Postcondition, over
  vstd's UTF-8 encoding specification.*
- `classify_verbatim_prefix` returns `Unc(8)` for bytes starting with
  `\\?\UNC\`, else `Plain(4)` for `\\?\`, else `NotVerbatim`.
  *Postcondition.* Each offset lies inside the input and ends an ASCII marker
  (`theorem_unc_precedence`, a lemma about the specification); that
  production's slice therefore starts on a character boundary is argued from
  UTF-8, not proved.

**Assumed, not proved**
- `kernel_component` and `kernel_prefix` copy each payload's encoded bytes and
  mirror the parsed `std::path::Prefix`, so kernel view equality is
  `Component` equality: from reading the Rust 1.90 sources, with pairwise
  unit tests.
- Production collects the kept components into a `PathBuf` and passes its
  re-parsed components to `classify_containment`. On Unix that round trip is
  taken to be the identity on normal forms, so the `spec_repo_relative`
  theorems, including the behavior-change lemmas, describe production. On
  Windows `PathBuf::push` can rewrite some lists; there the
  `logical_normalize` and `classify_containment` postconditions still hold
  for the lists each receives, but the composite theorems do not describe
  production.
- `Path::components` puts a prefix only first and yields non-empty `Normal`
  names other than `.` and `..`, and `as_os_str` of a `Normal` is its name.
  Production's indexing by the returned positions is in bounds by the
  postconditions, as lowering preserves length. None of the exec
  functions production calls has a `requires` clause.
- vstd specifies `str::as_bytes` as UTF-8. That `s@.contains` matches
  `str::contains(['\n', '\r', '\0'])` is by reading, with a unit test.
- The specifications transcribe the replaced code (review, plus a 2048-case
  differential property test on Unix in which only former panics differ, and
  a 13-input comparison with the former verbatim rule).

**Still unverified here**
- `Path::is_absolute`, `Path::join` and `Path::components` on the input and
  root, and the `PathBuf` collect and re-parse: `std::path` semantics, which
  Verus does not model here.
- The Windows `Path::to_str` gate (non-UTF-8 paths keep `\\?\`), the slicing
  that builds the rewritten path, UTF-8 conversion of accepted names, the `/`
  join, and error mapping: glue. The conversion and join could move into a
  kernel with a model of `OsStr::to_str` (WTF-8 on Windows).
- `RepoRelPath::to_path`: a path-level round trip needs a `Path::join` model.
- `from_git_bytes` checks only UTF-8 (`std::str::from_utf8`) and text, not
  the type's documented structural invariants, which it takes from Git;
  `from_normalized` checks nothing. A kernel predicate for those invariants
  could be added.

**Size:** 326 spec / 498 proof / 187 exec lines, 0 trusted. By file:
`path/model.rs` 32/1/72, `path/normalize.rs` 144/248/40,
`path/containment.rs` 94/117/40, `path/text.rs` 56/132/35; `path.rs` holds
only module documentation and `mod` declarations.

### Repository context, CLI policy and walk boundaries

**Production code:** `src/context.rs`, `src/cli.rs`, `src/walk.rs`.
**Kernels:** `waft_verified::{context, context::cli_policy, worktree_map,
boundary, boundary::gitfile}`.

From Git's answers (as identity keys), flags, path components, file names and
probe answers, these kernels decide which worktrees a command reads and
writes and which root error wins, where user configuration comes from, how
paths are re-rooted at the source, where the walkers stop at a nested
repository, and which target a `.git` file names.

**Verified decisions**
- Main worktree, roots, error precedence: `context::{select_main,
  resolve_roots}` in `resolve_context`.
- Default command, user-config source, `WAFT_*` layer gate, replace flag:
  `cli_policy::{command_kind, user_config_source, policy_env_layer_applies,
  cli_replace_extra_excludes}` in `Cli::command_kind`,
  `Cli::resolve_policy_for_context` (reader `ProcessConfigPath`) and
  `Cli::cli_layer`.
- Worktree stripped when re-rooting `info` arguments and the config-discovery
  start: `worktree_map::deepest_worktree_prefix`, over the paths kernel's
  `path::containment::component_prefix`, in `Cli::map_worktree_path_to_source`
  (called by `normalize_source_path` and `source_view_directory`).
- Control-file names, gitlink folding, nested-repository pruning, gitfile
  target: `boundary::{special_name_matches, boundary_case_folds,
  is_git_boundary}` in `walk::{special_filename_matches, boundary_case_folds,
  is_git_boundary_dir}`, and `gitfile::gitdir_target` in
  `read_dot_git_pointer`. `is_git_boundary_dir` passes the `FsBoundaryProbes`
  oracle and prunes the source walks in `src/git.rs`,
  `src/worktreeinclude_engine.rs`, `src/validate.rs` and
  `src/eligibility_groups.rs`.

**What the proofs establish**
- `select_main` returns the index of the first record flagged main,
  `BareMainCopy` iff the command is `copy` and that record is bare, and
  `NoMain` iff no record is flagged. `resolve_roots` returns
  `spec_resolve_roots`: source `--source`'s toplevel, else main (run from
  main, the working directory's observation of it); destination `--dest`'s
  toplevel, else, with neither flag, the working directory's worktree unless
  it is main, and none with only `--source`; error the first violated
  requirement of: `copy` has a destination, its source is main, source and
  destination differ, a `copy` destination is not main, the destination is in
  the family. Roots come back as observed; all comparisons are of identity
  keys. *Postconditions.*
- That error is the violated requirement of lowest rank; "copy destination is
  main" is never reported (production keeps the branch); non-`copy` commands
  get no copy-only error; a resolved destination differs from the source and
  is in the family; a resolved `copy` reads from main and writes into another
  family member; with neither flag, run in a family member other than main,
  every command resolves to main and that worktree, and run in main, `copy`
  needs a destination and the others resolve to main alone. If `main` is the
  first record flagged main and is not bare, a resolved `copy` writes into a
  different record of the list (another key); this says nothing about that
  record's own flags. *Lemmas about the specification. Production meets the
  last one's hypotheses by inspection: it passes the same records to both
  kernels and takes `main.key` from the record `select_main` chose, whose
  postcondition supplies the rest for `copy`.*
- `command_kind` maps no subcommand and `copy` to `Copy`, and each other
  subcommand to itself. `user_config_source` returns nothing under
  `--isolated` (even with `--config`), else the `--config` path, else the
  value of a set, Unicode, non-empty variable, else the default location, and
  calls the reader exactly once when neither flag decides, never otherwise.
  The kernel mirrors the existing behavior, including the default location
  for a non-Unicode value. `policy_env_layer_applies` is `!isolated`;
  `cli_replace_extra_excludes` is `Some(true)` with the flag and `None`
  without it, never `Some(false)`. *Postconditions, the read count over the
  reader's ghost log.* When a flag decides, the variable's value cannot
  change the result. *Lemma about the specification.*
- `deepest_worktree_prefix` returns `None` iff no worktree is a component
  prefix of the path, else the deepest, the last of equal depth (as
  `max_by_key` chose). *Postcondition.* The stripped components do not depend
  on the order or repetition of the list. A worktree matches with only names
  after it exactly when `spec_containment`, the decision
  `classify_containment` makes, is not `Outside`, and `Inside(n)` strips the
  same `n` components; the mapping itself, like `strip_prefix` before it, also
  accepts a remainder that is not all names. *Lemmas about the specification.*
- `special_name_matches` is byte equality, plus ASCII case-insensitive
  equality when `aliasing_platform` is set. A lemma shows that an ASCII
  case-insensitive match of an ASCII expected name is itself ASCII, which is
  why it agrees with the former `to_str`-based rule (also unit-tested against
  that rule). `boundary_case_folds` is `ignore_case || aliasing_platform`.
  `is_git_boundary` first prunes an entry named `.git` under that name rule,
  then keeps the walk root (depth 0), both without probing; otherwise it asks
  whether `.git` is a directory, then a gitfile with an existing target, then
  whether the path is a gitlink (folding per `boundary_case_folds`), stops at
  the first yes, and prunes iff one answered yes. *Postconditions, the probes
  over the oracle's ghost log, and lemmas about the specification.*
- `gitdir_target` returns the remainder of the first line that starts with
  `gitdir:`, trimmed of whitespace, or `None` if that is empty or no line
  qualifies. The kernel mirrors the existing parse, including a tag line
  anywhere in the file. *Postcondition over `line_at` and `trim`, hand-written
  models of `str::lines` and `str::trim` over vstd's `is_white_space`.
  Nothing mechanically ties them to std; a 23-input unit test compares the
  kernel with the former std-based parse.*

**Assumed, not proved**
- `PathIdentities` gives equal keys exactly to `Path`-equal paths
  (unit-tested). Records are `list_worktrees` output in order; each toplevel
  is `show_toplevel`'s answer for the argument its variant names; both
  backends flag only the first record as main (proved for the CLI parser by
  `git_parse::parse_worktree_list`, by construction in the gix backend).
  `Cli::command_kind`'s subcommand match and the `CommandKind` conversions in
  `src/context.rs` mirror the enums (unit-tested).
- `ProcessConfigPath` classifies `std::env::var` faithfully; `FsBoundaryProbes`
  answers from the filesystem and the gitlink set (via
  `RepoRelPath::normalize` and `git::repo_paths_equivalent`). Both trait
  contracts only log calls.
- `kernel_path_components` is faithful (paths section); `Path::strip_prefix`,
  re-run on the chosen worktree, succeeds exactly on a component prefix
  (tested against the kernel and against the former mapping).
- `as_encoded_bytes` equality is `OsStr` equality; `ALIASING_PLATFORM`
  (macOS, Windows) names the case-aliasing platforms; walkers pass true
  depths; vstd's specification of `char::is_whitespace` matches std's.
  `walk::entry_kind` (no-follow `FileType` to `selection::EntryKind`, for the
  rules and Git kernels) is unit-tested on Unix. No exec function production
  calls here has a `requires` clause.

**Still unverified here**
- Working directory, `-C`, the probe path and the Git query order, which puts
  Git and `select_main` errors before root errors: I/O glue; the query plan
  could become a kernel behind an oracle.
- `PathIdentities` (could move into a kernel behind a path-equality oracle),
  mapping verdicts to `Error`, clap, `effective_directory`,
  `canonicalize_parent`, and the `strip_prefix` re-run and join: glue over
  std.
- Loading the chosen config file (a `--config` or variable path is resolved
  against the `-C` directory), and calling `user_config_path` only for
  `DefaultLocation`: glue; no theorem links `cli_policy` to
  `policy::user_source`. The read count covers `ProcessConfigPath` only; when
  not isolated, `layer_from_env` also enumerates the whole environment, and
  its key table ignores `WAFT_CONFIG_PATH` (configuration section).
  `dispatch` defaults to `copy` by its own match, which agrees with
  `command_kind` by inspection; the `subcommands::run_*` library entry points
  pass a fixed `CommandKind` instead.
- Probe answers, gitfile reading and target resolution, and which entries
  each walker passes: I/O glue.

**Size:** 566 spec / 565 proof / 371 exec lines, 0 trusted. By module:
`context` with `cli_policy` 282/265/217, `worktree_map` 48/62/27, `boundary`
with `gitfile` 236/238/127; `component_prefix` counts under paths.

### Configuration policy

**Production code:** `src/config.rs`. **Kernels:**
`waft_verified::policy::{vocab, merge, raw, env_layer, user_source, discovery}`.

The kernels decide the knobs' spellings and presets, the layer order and
merge, config-file acceptance, the `WAFT_*` variables and their values, the
user config's base variable, and which directories below the source root are
searched for `.waft.toml`. The `vocab` enums are the only kernel copies of
`SymlinkPolicy`, `WorktreeincludeSemantics` and `WhenMissingWorktreeinclude`:
`selection` takes all three and `git_policy` and `validation` take
`SymlinkPolicy` directly, so each of these knobs reaches every kernel through
the one `From` conversion in `src/config.rs`.

**Verified decisions**
- Spellings and the `a|b|c` hint: `vocab::<Enum>::{as_str, parse, EXPECTED}`
  in each production enum's `as_str`, `parse` and `unknown_value`.
- Order and merge: `merge::resolve_inputs` in `PolicyResolutionInputs::resolve`
  (called from `src/cli.rs`), which runs `resolve_layers` itself;
  `merge::{resolve_layers, default_knobs}` behind
  `ResolvedPolicy::{from_layers, default}`, which only tests call;
  `merge::selects_without_rule_file` in `CompatProfile`'s method of that name.
- `raw::layer_from_raw` in `parse_toml`; `env_layer::{EnvKey::classify,
  parse_env_bool, parse_exclude_list}` in `layer_from_env_iter` (the boolean
  through the wrapper `parse_env_bool`); `user_source::select_config_home` in
  `user_config_path`; `discovery::walk_toward_invocation` in
  `discover_project_configs_in_repo`. `context::cli_policy` (context area)
  decides whether `--isolated`, `--config` or `WAFT_CONFIG_PATH` overrides the
  default user config location, and whether the environment layer applies.

**What the proofs establish**
- `as_str` returns the canonical name; `parse` returns `spec_parse(s)`: a
  variant spelled exactly `s` (case-sensitive, untrimmed), for `CopyStrategy`
  also `default`, `simple`, `cow`, `reflink`, else `None`. *Postconditions.*
  No spelling names two variants, parsing round-trips, and `EXPECTED` joins
  the canonical names with `|` in declaration order. *Lemmas about the
  kernel's specification (`theorem_*_vocabulary`).*
- `resolve_layers` returns the closed form: the last explicit profile, else
  `claude`; each profile-owned knob is its last explicit setting at or after
  the last profile selection, else the resolved profile's preset;
  `copy_strategy` is the last explicit one, else `auto`; the extras are every
  layer's extras, in layer order, from the last layer with
  `replace_extra == Some(true)` on (from the first when none has it).
  `resolve_inputs` returns it for defaults, user, the project layers in the
  order given, environment, CLI (production passes the project layers
  outermost first, in discovery's order). `selects_without_rule_file` is
  whether the profile's preset has `when_missing = all-ignored`.
  *Postconditions.*
- The closed form equals folding `apply_layer`, a hand restatement of the rule
  documented on `from_layers` (`theorem_closed_form_is_fold`). P1-P8: the last
  explicit profile wins; nothing below the last profile selection but
  `copy_strategy` matters; `copy_strategy` depends only on its own fields;
  `replace_extra = false` acts as unset; a layer that sets nothing changes
  nothing; only `wt`'s preset selects without a rule file; explicit CLI
  values win, a CLI profile sets the owned knobs the CLI leaves unset to its
  preset, an environment value of a profile-owned knob wins unless the CLI
  sets that knob or a profile, and a CLI replace keeps only the CLI's extras
  (`theorem_precedence`, silent on user, project and defaults against each
  other); a profile-owned knob that differs from the resolved profile's preset
  was set to that value at or after the last profile selection, by a layer the
  theorem does not identify (`theorem_attribution`). *Lemmas about the
  kernel's specification.*
- `layer_from_raw`: a present `version` other than 1 is `UnsupportedVersion`
  whatever else the file holds, and only then; otherwise the file is accepted
  exactly when every present field is an accepted spelling, an error names
  the first field in a fixed order that is not, with its raw value, and an
  accepted layer holds each field's parse and `replace_extra` unchanged.
  *Postconditions; `theorem_first_bad_field` makes them determine the result.*
- `classify(k)` is the key named exactly `k`, else `None`; the eight names are
  distinct and start with `WAFT_` (`theorem_env_key_table`). `parse_env_bool`
  trims Unicode `White_Space` and ASCII-lowercases, then reads `1|true|yes|on`
  as true, `0|false|no|off` or empty as false, anything else as `None`.
  `parse_exclude_list` splits on `,`, trims, drops empty pieces; every glob is
  non-empty, trimmed, comma-free and a contiguous part of the value
  (`theorem_exclude_list_globs`). *Postconditions; lemmas about the kernel's
  specification.*
- `select_config_home` queries `XDG_CONFIG_HOME`, `HOME`, `USERPROFILE` in
  order, stops at the first the oracle reports set and returns it, reading no
  later one; `None` means all three were queried and none was set.
  `walk_toward_invocation` enters the name components before the first `..`,
  root or prefix component (skipping `.`), in increasing order and at most
  once each, searches each one that is not a boundary right after entering
  it, and stops at the first boundary without searching it; the kernel
  mirrors the existing behavior of keeping directories searched before a `..`.
  *Postconditions over the oracles' call logs, for every answer; with
  `theorem_reachable_names`, a lemma about the kernel's specification.*

**Assumed, not proved**
- The six enums' `From` impls, `ConfigLayer::{kernel_view, from_kernel}`,
  `ResolvedPolicy::from_kernel`, `unknown_value_message` and the `EnvKey` match
  map each variant or field to its counterpart; `component_kind` maps `.`,
  names, and `..`/root/prefix to `CurDir`, `Normal`, `Stop`; the
  `ConfigHomeVar` match picks the path suffix. All are exhaustive; unit tests
  check the round trips, that clap's value names equal `as_str`, and the
  environment key names.
- The oracles log every call; the theorems hold for any answer, and that the
  answers are right is production's. `ProcessConfigHome` answers
  `std::env::var_os(..).is_some()`, as the replaced code did, and keeps the
  last queried value, which `user_config_path` uses.
  `ConfigSearchWalk` keeps the root joined with the entered names, probes it
  (`.git` exists, or `git::repo_paths_equivalent` matches a gitlink after
  `RepoRelPath::normalize`) and records it on `search`. No called kernel has a
  `requires` clause.
- serde/toml reject unknown keys and wrong types. vstd's specifications of
  `&str` and string literals, `Chars`, slices, `char::is_whitespace`,
  `to_ascii_lowercase`, `Vec` and `String` are correct. `split_commas`, `trim`
  and `ascii_lowercase` follow `str::split(',')`, `str::trim` and
  `str::to_ascii_lowercase` by reading; a property test checks
  `parse_exclude_list` against std, a unit test checks `parse_env_bool`
  against std on examples, and property tests check both merges against a
  copy of the replaced merge.

**Still unverified here**
- The loop over `std::env::vars()` (last occurrence wins, first error returns,
  field assignment): a kernel consuming entries one at a time could take it
  with more work; collecting them first would change the order of reads and
  errors, so it stays glue. The legacy `discover_project_configs`, called
  only by tests, could use the discovery oracle pattern.
- TOML deserialization and clap's flag parsing: outside Verus's reach here.
- File loading, discovery's path work (`strip_prefix`, the root always
  searched, the `.waft.toml` join and filter), path suffixes, message text:
  I/O and formatting glue.

**Size:** 1046 spec / 1400 proof / 578 exec lines, 0 trusted (`line_count`
counts the two oracle traits as 5 spec and 6 exec lines). By file under
`policy/`: `vocab` 214/279/148, `merge` 449/580/148, `raw` 102/99/97,
`env_layer` 179/299/118, `user_source` 26/21/31, `discovery` 76/122/36;
`policy.rs` is documentation.

### Validation, lint and list/info reports

**Production code:** `src/validate.rs`, `src/subcommands/{copy, list, info,
validate}.rs`, `ValidationReport` in `src/model.rs`. **Kernels:**
`waft_verified::{validation, lint, report_view}`.

The kernels decide problem severities, what rule-file discovery does with a
walked entry, the validator's skip rule and the lint findings, which issues
each command prints and whether it stops, and the labels of `list -v` and
`info`. The action labels are proved against the planner kernel's decision
specification; the `source_kind` label only against a hand-written model.

**Verified decisions**
- `validation::issue_severity` through `severity_of` at every push site, and
  `validation::rule_file_action` in `discover_and_validate`, over
  `policy::vocab::SymlinkPolicy`, the enum `selection` and `git_policy` take.
- `lint::is_blank_or_comment` (the skip rule, on the trimmed line) in
  `validate_ignore_file`; `lint::lint_lines` in `check_suspicious_patterns`.
- `validation::validation_view` (calling `has_errors` and `error_count`) in
  `report_preflight`, which `copy`, `list` and `info` call, and in
  `run_validate_with_context`. `ValidationReport::{has_errors, error_count}`
  wrap the same kernels; no command calls them, only unit tests.
- `report_view::list_action` (through `predicted_action`) in
  `run_list_with_context`; `report_view::{source_kind, info_destination}` and
  `SourceKindLabel::{exists, is_regular_file, is_directory}` in
  `run_info_with_context`.

**What the proofs establish**
- `shown` lists, in increasing order, exactly the issues `spec_shown` selects:
  for a preflight, the errors if any, else all issues unless `--quiet`; for
  `waft validate`, errors always and warnings unless `--quiet`. The outcome
  is `Abort` with the exact error count iff some issue is an error;
  `validation passed` is announced iff `waft validate` runs without `--quiet`
  and finds no error. *Postconditions of `validation_view`.* The outcome is
  the same for every command and `--quiet`
  (`theorem_gate_is_mode_independent`). *Lemma about the specification.*
- A problem is a warning iff it is a lint finding or concerns the global
  excludes file. *Postcondition of `issue_severity`.* So, given the tags
  production assigns, a command stops at validation (and `waft validate`
  fails) iff some problem is a non-lint `Repository` problem
  (`theorem_only_repository_problems_block`). *Lemma about the
  specification.*
- For a walk entry's no-follow type (never both symlink and file), a symlink
  is read only if it is a `.worktreeinclude` under `follow`, and rejected
  unread exactly if it is one under `error`
  (`theorem_symlinks_read_only_when_followed`). For each no-follow
  `selection::EntryKind` of a `.worktreeinclude`, discovery validates or
  rejects the entry exactly when `git_policy::rule_file_counts_spec` counts
  it as existing, that is, when the wt walk reads it
  (`selection::spec_wt_entry_consulted`); it rejects exactly a symlink under
  `error`, which the wt walk would read, with an error-severity issue
  (`theorem_rule_file_action_agrees_with_existence`).
  *Postcondition of `rule_file_action`; lemmas about the three kernels'
  specifications.*
- A trimmed line is skipped iff it is empty or starts with the byte `#`.
  `duplicate_of == Some(j)` iff line `i` is not blank or a comment and `j` is
  the least earlier line with byte-equal trimmed text; `shadowed_by ==
  Some(d)` iff line `i` starts with `!` and `d` is the least earlier literal
  directory exclusion (not a comment or negation, ends in `/`, no `*`, `?`,
  `[`) that prefixes line `i` without its `!`. *Postconditions of
  `is_blank_or_comment` and `lint_lines`.* Verdicts are unique, blank and
  comment lines are never flagged or named, and on UTF-8 (vstd's
  `encode_utf8`) the byte tests equal the `&str` tests used before
  (`theorem_bytes_agree_with_chars`). *Lemmas about the specification.* The
  kernel mirrors the existing heuristic: every test is made on the line
  production trimmed at both ends, and the shadow test is a byte-prefix test.
- For every `planner::entry::EntryWorld` and both flags, the table
  `spec_plan_outcome` is `planner::entry::plan_entry_spec`'s outcome up to
  the kind of copy or failure (`theorem_model_is_planner`). For a regular
  source, a clear parent re-check and a successful source snapshot, the
  predicted label announces `plan_entry_spec`'s outcome without
  `--overwrite`, never a failure, and (with a successful destination
  snapshot) a skip label sets the remedy flag iff `--overwrite` plans a copy
  (`theorem_prediction_is_planner_decision`). `planned_action` appears iff
  the path is eligible and its destination was classified, and then equals
  the label `list -v` gives a regular source in the same state
  (`theorem_info_view`). *Postconditions of `predicted_action`,
  `list_action` and `info_destination` (each returns its specification);
  the planner theorems are lemmas relating that specification to the
  planner kernel's, `theorem_info_view` one about `report_view`'s own.*
- `source_kind` is `file` iff `spec_source_exam` finds a regular file, and
  `missing` iff it cannot examine the source
  (`theorem_source_kind_matches_planner`); `exists`, `is_regular_file` and
  `is_directory` test for not `missing`, `file` and `directory`.
  *Postconditions; lemma about `spec_source_exam`, a hand-written model of
  `RealFs::source_kind` in `src/fs.rs`; nothing mechanically ties the two,
  and `info` makes its own `lstat`.*

**Assumed, not proved**
- Each push site passes the right `(RuleScope, IssueKind)`; production prints
  `issues[k]` for `k` in `shown` and acts on the outcome. The conversions
  `ValidationReport::severities`, `validation_severity`, `dest_state_view`
  and the `SymlinkPolicy` `From` are unit-tested exhaustive matches;
  `LstatType` is copied field by field from one `symlink_metadata` call and
  has no unit test of its own. No called exec function here has a
  `requires` clause.
- walkdir's entry type does not follow links, so `is_symlink()` and
  `is_file()` agree with `walk::entry_kind`. The symlink theorems are per
  entry: that validation's walk visits the entries the existence and wt
  walks visit is not proved, and `.git/info/exclude` and the global excludes
  file are opened by path, through symlinks, under every policy.
- Lint input is `line.trim()` for each of `content.lines()`; std's `lines`
  and `trim` and vstd's model of `&str` bytes are trusted. A property test
  compares the warnings with a copy of the pre-kernel code.
- Nothing changes between the observations a prediction combines, which
  `list -v` and `info` make in the same order as before; both classify with
  the planner's own `planner::classify_destination` and pass
  `fs::overwrite_supported()`, as the planner does. The string tables print
  a remedy only when its flag is set. That `src/planner.rs` executes
  `plan_entry_spec` is the planner area's claim.

**Still unverified here**
- Which files are validated (the walk's name match and nested-repository
  filter are the `boundary` kernel's, in its own area), the per-site tags,
  and the path choice in `find_global_excludes`: plain code that could move
  into a kernel. The config, environment and existence reads are I/O glue.
- Pattern validity in `ignore`'s `GitignoreBuilder`: a third-party crate,
  outside what Verus can reach here.
- The walk, file reads, line splitting and the probes of `list -v` and
  `info`: I/O glue. How `info` combines its eligibility answer (its path
  comparisons go through the git area's `tracked` kernel) and its gitignore
  label: small logic that could move into a kernel.
- Output text, including `SkipReason::describe` for `copy --dry-run`, which
  runs parallel to the remedy flag without using it: formatting glue.

**Size:** 666 spec / 462 proof / 440 exec lines, 0 trusted. By file:
`validation.rs` 153/142/142, `lint.rs` 168/250/122, `report_view.rs`
345/70/176. Proofs also use planner,
`selection`, `git_policy` and `policy::vocab` items counted with their areas.

### Git backend

**Production code:** `src/git.rs`. **Kernels:** `waft_verified::tracked`,
`waft_verified::git_parse`, `waft_verified::git_policy` (whose
`rule_file_counts` calls `waft_verified::selection::wt_entry_consulted`).

Both backends decide whether a repository path is tracked with one verified
lookup, which fixes the answer and the exact identity probes behind it. The
CLI backend reads `ls-files -s`, `worktree list --porcelain`,
`check-ignore -v -n`, `ls-files --others` (all `-z`) and single-line
`rev-parse` output with verified parsers. Policy kernels pick the backend,
the case policy and `--bool` answers, spot a missing-repository message, run
the index snapshot cache step, and decide which walked entries count as an
existing `.worktreeinclude`.

**Verified decisions**
- Trackedness and the paths stat-ed for it: `TrackedLookup::select` in
  `TrackedPathLookup::select`, under both backends' `tracked_paths` (and
  `GitGix::check_ignore`, which reports tracked paths as not ignored);
  `TrackedLookup::contains` in `TrackedPathLookup::contains`, the per-entry
  check of the gix walkers and `cli_list_candidates_with_engine`; the lookup
  comes from `TrackedLookup::new` in `TrackedPathLookup::new`.
- `tracked::{utf8_fold_key, non_utf8_fold_key}` in `case_folded_repo_path`;
  `tracked::paths_equivalent` in `repo_paths_equivalent` (gitlink checks in
  `src/walk.rs` and `src/config.rs`, eligibility matching in `info`);
  `tracked::alias_resolves_to` in `RepoPathAliasProbe::resolves_to` (`info`).
- `git_parse::{parse_ls_files_stage, parse_worktree_list,
  parse_check_ignore, parse_nul_list, line_without_ending}` in
  `RepoIndexState::from_ls_files_stage_output`, the production
  `parse_worktree_list`, `parse_check_ignore_output`,
  `GitCli::list_ignored_untracked` and `trim_git_line_ending`
  (`GitCli::{show_toplevel, resolve_index_path}`).
- `git_policy::select_backend` in `select_git_backend`;
  `case_folding_applies` in `git::case_folding_applies` (both backends);
  `parse_bool_output` in `GitCli::{read_checkout_folds_case,
  read_bool_config}`; `mentions_missing_repository` in `cli_discovery_error`;
  `index_state` in `RepoStateCache::index_state` (effects:
  `IndexStateEffects`); `rule_file_counts` in
  `walk_for_first_worktreeinclude` (`worktreeinclude_exists_anywhere`).

**What the proofs establish**
- `contains` returns `is_tracked`, the naive-scan specification: the query
  is an entry, or an entry shares its fold key and either the checkout folds
  case or the query resolves to an identity such an entry also has. The
  probe log grows by exactly `probe_trace`: nothing for an exact entry, no
  collision, or a case-folding checkout; else the query, then (if it
  resolves) colliding entries in index order up to the first match. `select`
  returns exactly the ascending positions of tracked queries, with each
  `probe_trace` in query order. Neither depends on the hash or its seed.
  *Postconditions, via `TrackedLookup`'s type invariant (keys are the
  entries' folds; each `KeyTable` bucket holds exactly the positions hashing
  to it) and `lemma_positions_of_exact`.*
- For every fold function and filesystem: an exact entry is tracked without
  a probe; on a case-folding checkout a collision suffices and nothing is
  probed; on a case-sensitive one a non-exact query needs a resolved identity
  that a colliding entry shares; folding only adds protection; only queries
  and entries folding onto them are probed. *Lemmas about the kernel's
  specification (`theorem_exact_spelling_always_tracked`,
  `theorem_case_sensitive_requires_identity_evidence` and five more).*
- Fold keys are tag 0 plus production's UTF-8 fold, or tag 1 plus the
  ASCII-lowercased bytes; the kinds never collide. `paths_equivalent` is byte
  equality, or fold-key equality when the checkout folds case; it is an
  equivalence relation. `alias_resolves_to` holds iff the alias identity
  production passes is `Some`, the spellings differ and `canonical` has that
  identity; it stats `canonical` once when the identity is `Some` and the
  spellings differ, and otherwise not at all. *Postconditions; the
  never-collide and equivalence facts are lemmas about them
  (`theorem_fold_key_kinds_never_collide`, `theorem_equivalence_relation`).*
- The four `-z` parsers each equal a grammar over the NUL split
  (`theorem_nul_fields_is_the_split`); all returned spans are in bounds.
  ls-files: an entry per field with a TAB, the path after the first TAB, a
  gitlink iff the metadata starts with `160000 `. Worktrees: one record per
  `worktree ` field, the first main, bare iff its body has `bare`; a `bare`
  outside a record marks the next. check-ignore: groups of four fields,
  leftovers dropped; a match unless source and line number are both empty;
  ignored iff matched and the pattern lacks a leading `!`; the line as
  `usize_from_str` reads it, else 0. Path lists: the non-empty fields.
  `line_without_ending` drops one LF, then one CR. *Postconditions, plus
  `theorem_one_record_per_worktree_field`, a lemma about the worktree
  grammar. That the grammars keep the replaced code's handling of malformed
  output rests on review and differential tests, not proof.*
- For well-formed records (NUL-free fields, plus each theorem's shape
  conditions, such as a space-free mode or attribute fields that do not
  start with `worktree `), parsing the bytes a hand-written model of Git's
  `-z` writers produces returns exactly each record's path and flags, in
  order (`theorem_ls_files_round_trip`, `theorem_worktree_list_round_trip`,
  `theorem_check_ignore_round_trip`). *Lemmas about the specification;
  nothing ties the writer model to Git.*
- Unset `WAFT_GIT_BACKEND` is gix, non-Unicode is an error, and a trimmed
  value selects gix or cli iff its ASCII lowercase is that name, else is an
  error. An explicit `core.ignoreCase` decides; only an absent key defers to
  the platform. `parse_bool_output` accepts exactly `true` and `false`.
  `mentions_missing_repository` is contiguous substring search for
  `not a git repository`. *Postconditions, with `theorem_backend_selection`
  and `theorem_configured_case_policy_wins`.*
- `index_state` performs exactly `cache_call_trace` for the answers its
  effects returned: stat; if that fails, build and return the build's
  result; else serve the slot iff it was stored under exactly that stamp;
  else build (a failed build ends the call), stat again, and store under the
  first stamp iff the second matches. *Postcondition over the `IndexCacheIo`
  ghost log.* Over any sequence of calls on one repository, with the index
  changing arbitrarily, a call that builds nothing returns a snapshot an
  earlier call built and stored between two stats giving this call's first
  stamp. *`theorem_cache_serves_only_attributable_snapshots`, over a
  hand-written slot model (it starts empty and a read returns the last
  store); nothing mechanically ties `RepoStateCache`'s map to it.*
- `rule_file_counts` is the name match and selection's wt consult gate, over
  the shared `selection::EntryKind` and `policy::vocab::SymlinkPolicy`: a
  name-matching regular file counts, a name-matching symlink unless the
  policy is `ignore`, nothing else. When the walk's entry type agrees with
  the per-path probes (`consistent_entry_facts`), every rule file the
  per-path engines read exists, and one that exists but they skip is a
  symlink to a non-regular file under `follow` or `error`. *Postcondition,
  and lemmas about the `git_policy` and `selection` specifications
  (`theorem_rule_file_existence`,
  `theorem_rule_file_existence_agrees_with_selection`).*

**Assumed, not proved**
- Oracle contracts, unchecked under rustc: `UnicodeCaseFolding` is a
  deterministic function of the bytes (exact entries are protected without
  it); `FilesystemIdentityProbe` gives lstat `(dev, ino)` on Unix, volume
  serial and file index on Windows, `None` elsewhere, stable during a call;
  `IndexStateEffects` stats when asked (an lstat plus a separate identity
  probe), builds from the index as it stands, and its slot returns the last
  store (production is single-threaded). The ghost logs stand for the real
  calls.
- Entries are the index's spellings (from `parse_ls_files_stage` or gix's
  index reader); Git writes its documented `-z` formats and reports a
  discovery miss with `not a git repository`; `usize_from_str` is
  `str::parse::<usize>`, by reading and the check-ignore differential test.
  `index_stamp` is injective (by construction, spot-checked by a unit test)
  and equal stamps are assumed to mean equal index contents, because Git
  replaces the index by rename.
- Glue: span slicing, mapping `select`'s positions back to paths,
  `RepoRelPath::from_git_bytes`, `path_buf_from_git_bytes` and lossy UTF-8 on
  parser output; `str::trim`, `str::to_lowercase`, lossy UTF-8 and the config
  defaults around the policy kernels (a git error or unrecognized output is
  "unset" in `read_checkout_folds_case`, `false` in `read_bool_config`);
  gix's boolean parsing; `cfg!(any(target_os = "macos", windows))`;
  `walk::entry_kind` and the `vocab::SymlinkPolicy` conversion (the name
  test, `walk::special_filename_matches`, is the verified
  `boundary::special_name_matches`, covered with walk boundaries); the alias
  identity (`RepoPathAliasProbe::new`: an exact-spelling directory listing,
  then a stat).
- The specifications transcribe the replaced code, by review. Nine 512-case
  differential property tests compare the fold, the lookup, the four `-z`
  parsers, the line trim, backend selection and the discovery phrase with
  test-only copies of that code or `std`, and a Unix hard-link test compares
  identity checks; the cache step, case policy, `--bool` reading, alias check
  and existence gate have no differential test. No exec function production
  calls has a `requires` clause.

**Still unverified here**
- Spawning `git`, exit statuses, the `check-ignore` stdin encoding, stats and
  path conversions: I/O glue; the encoding could move into a kernel.
- The gix backend's discovery, `show_toplevel`, `list_worktrees`,
  `check_ignore` matching, config reads and index extraction: gix types,
  outside what Verus reaches here.
- The walkers' candidate pipeline and the error-skipping
  `walk_for_first_worktreeinclude` walk (their nested-checkout filter is
  `boundary::is_git_boundary`), and `RepoStateCache`'s first-answer memos:
  walkdir and locks are I/O; the sequencing and memo steps could move into
  kernels over effects traits, as `index_state` did.
- The Unicode fold (NFD, case fold, NFD): table-driven library code, beyond
  Verus here. `read_config`, config error defaults, the empty-worktree-list
  error and messages: small glue.

**Size:** 1310 spec / 1493 proof / 659 exec lines, 0 trusted. By file:
`tracked.rs` 481/586/235, `git_parse.rs` 530/772/247, `git_policy.rs`
299/135/177. The `selection` gate is counted under the rules area.

### Rule engine

**Production code:** `src/worktreeinclude.rs`, `src/worktreeinclude_engine.rs`,
`src/policy_filter.rs`, `src/subcommands/mod.rs`. **Kernels:**
`waft_verified::rules::{precedence, lines, reference}`,
`waft_verified::exclusion`, `waft_verified::selection`.

The kernels decide the `git` engine's verdict for a path from its rule files'
matcher answers (both negation caveats, the reported file and glob, the order
of matcher queries) and the `claude-2026-04` verdict from the root file; which
rule lines become patterns, the line lookup behind `info` and `list -v`, and
the wt-0.39 negation grammar; whether a rule file is read; whether the
exclusion filter drops a candidate and whether it folds case; where
candidates come from; and which cause an empty selection reports. Glob
matching stays in the `ignore` crate behind oracle traits. The kernels were
written to mirror the replaced code (checked by review and tests, not by
proof), not Git; a hand-written model, `rules::reference`, states where the
two agree and where they differ.

**Verified decisions**
- `precedence::explain` in `explain_path` (the `git` engine, also used by
  `Wt039Semantics` to explain one path) and `precedence::explain_root_only`
  in `evaluate_root_only_path` (`claude-2026-04`), both answered by
  `ContextMatchers`; reached from the Git backend's per-path candidate
  discovery (which asks about files only), `info` and `list -v`.
- `lines::rule_line_pattern_len` in `build_file_match_context`,
  `lines::last_line_with_text` in `glob_line_info`, `lines::wt_negation_target`
  in `collect_wt_literal_negations`.
- `selection::rule_file_consulted`, through the private wrapper
  `rule_file_consulted`, in `applicable_contexts` and
  `root_rule_file_is_consulted` (which `evaluate_root_only_path` and the
  diagnosis use); `selection::wt_entry_consulted` in
  `collect_wt_literal_negations` and, through `git_policy::rule_file_counts`,
  in the existence walk (Git backend section).
- `exclusion::path_excluded` in `path_excluded` (`src/policy_filter.rs`),
  answered by `ExclusionLevels`; `exclusion::exclusion_folds_case` in
  `effective_case_insensitive`.
- `selection::selection_source` in `select_candidates`;
  `selection::diagnose_empty_selection` in `diagnose_empty_selection`, called
  by `eligible_records` when no candidate survives exclusion or no eligible
  record remains.

**What the proofs establish**
- `explain` returns `spec_explain` of the oracle's answers: the deepest file
  with a path verdict decides. A selecting verdict is `Included`. A negation
  yields to the deepest ancestor directory the same file selects, else to the
  first ancestor selection of any file (deepest ancestor, then shallowest
  file), else is `ExcludedByNegation`. No path verdict is `NoMatch`. The query
  log is exactly `explain_trace`, a spec written to follow the replaced code's
  query order. `explain_root_only` does the same for the root file alone,
  with only that file's queries (none, and `NoMatch`, without a file).
  *Postconditions, over the oracle's answers.*
- `NoMatch` exactly when no applicable file matches; shallower path verdicts
  never matter once a deeper file matches; a negation deselects exactly when
  it is the deepest file's verdict and no file selects any ancestor; the
  reported file and glob come from a query that returned them; with one file
  the cross-file caveat is a no-op, which is why `explain_root_only` skips
  it. *Lemmas about the kernel's specification.*
- `rules::reference` is a hand-written model. A path of `len` components has
  levels `0..=len` (level `k` is its first `k` components, level 0 the
  repository root), and a table `own(i, k)` gives rule file `i`'s verdict at
  level `k` as `ignore` computes it, including levels at or above the file's
  own directory (which `ignore` sees as the empty path or as an unstripped
  outer path) that Git never asks about. From the table the model derives the
  oracle answers (`model_pm`: the walk of `matched_path_or_any_parents` from
  the path up to the file's directory; `model_ai`: ancestor `a` is level
  `len - 1 - a`) and Git's verdict (`git_selects`: some level `k >= 1` whose
  deepest file living above it with a verdict there says `Ignore`). Git's
  side reuses the same table. With only the root file (either engine: both
  postconditions then give `spec_explain` over one file), the modeled verdict
  selects a path exactly when Git does, or when the root file has no verdict
  on levels `1..=len` but selects level 0, a case Git never selects
  (`theorem_root_file_divergence_is_the_empty_path`). With any number of
  files and no negated verdict at any level, the same holds with "some file
  selects its own directory"
  (`theorem_without_negations_divergence_is_the_empty_path`). Each has a
  plain-agreement corollary. Five `lemma_quirk_*` lemmas exhibit tables, each
  with a negation or such a line, where the two differ; the kernel mirrors
  that existing behavior. *Lemmas about a hand-written model of `ignore`
  0.4.33 and of Git's per-directory exclude semantics; they describe
  production only where `ContextMatchers` answers as `model_pm` and
  `model_ai` say, and Git only where Git matches each line at each level as
  `ignore` does and combines levels as `git_selects` says. Nothing
  mechanically ties the model to `ignore` or to Git. The test
  `worktreeinclude::tests::reference_tables_match_ignore` pins the five
  lemmas' six concrete tables (the `*/` lemma at two lengths) to `ignore`: it
  recomputes each row with `Gitignore::matched` on the real matchers, checks
  that `ContextMatchers`' path and ancestor answers equal the model's, and
  that `precedence::explain` on the real matchers selects exactly as the
  lemma says. No test runs Git on these inputs or checks `git_selects`;
  `prop_waft_matches_git_oracle` in `tests/property.rs` compares the `git`
  profile's end-to-end selection with `git ls-files
  --exclude-per-directory=.worktreeinclude` on generated patterns, which
  tests production, not the model.*
- `rule_line_pattern_len` is `None` exactly when the line starts with `#` or
  is empty once trailing `char::is_whitespace` characters are removed, else
  the trimmed length. `last_line_with_text` returns the greatest index whose
  text equals the glob's original text, `None` only if none does.
  `wt_negation_target` trims whitespace at both ends and returns a range
  exactly when the text starts with `!` and has no `*`, `?`, `[` or `]` after
  it; the range drops one leading `/` if present. *Postconditions.*
- `path_excluded` returns whether the first level with a verdict (the path,
  each parent, then the empty path) is a selection, asking the levels in
  order and none past it; `exclusion_folds_case` is
  `core.ignoreCase || platform may alias`. *Postconditions.* Without negated
  matches, a candidate is excluded iff some level is selected. Below an
  excluded directory every path is excluded unless a negated pattern matches
  it or a directory between them; the exception follows from the level order
  the kernel keeps from the replaced code. *Lemmas about the specification.*
- `selection_source`: with a rule file, `WtSubtractive` under `wt-0.39`, else
  `PerPathEngine`; without one, `when_missing` alone picks `Blank` or
  `AllIgnored`. `rule_file_consulted` holds iff the location resolves to a
  regular file and, under `ignore`, is not itself a symlink; it probes for a
  symlink only under `ignore` and only after finding a regular file.
  `wt_entry_consulted` admits a regular file, and a symlink unless the policy
  is `ignore`. *Postconditions, `rule_file_consulted`'s with the probe log.*
  On consistent entry facts the wt gate is never stricter, and differs only
  for a symlink whose target is not a regular file under `follow` or `error`,
  which only wt reads. *Lemma about the specifications.*
- `diagnose_empty_selection` returns `spec_diagnosis`, a decision table. It
  probes the root symlink exactly under `ignore`, the root gate exactly for a
  found rule file under root-only semantics whose root symlink was not
  skipped, and runs the fallible `follow` walk exactly for a blank selection
  without rule files under `ignore`. *Postcondition, with the probe log.*
  Each reported cause implies the observed facts behind its message, a
  symlink is blamed only under `ignore`, an error comes only from the walk,
  and nothing is reported when a rule file was found and the semantics is
  not root-only (`theorem_diagnosis_soundness`). *Lemma about the
  specification.*

**Assumed, not proved**
- `ContextMatchers` answers each file with
  `Gitignore::matched_path_or_any_parents` on the root-joined path (a verdict
  that may come from a line matching an ancestor directory) and with the glob
  of an ignore match of `matched(ancestor, true)`; matchers are pure.
  Production picks the files (one per directory from the root to the path's
  parent that passes the gate, reads and compiles; others are skipped) and
  the ancestor list (`Path::parent` steps, parent first, empty path
  excluded).
- `ExclusionLevels` asks `matched` per `Path::parent` level (the path with
  its `Path::is_dir`, which follows symlinks; every parent as a directory);
  that equals `ignore` 0.4.33's own walk only because its prefix strip is the
  identity on a candidate without a root or leading `.` component under an
  absolute matcher root (`root_strip_is_identity`, true for the CLI).
  Otherwise one level holds `matched_path_or_any_parents` itself and the
  theorems cover that level only. `level_walk_matches_ignore_parent_walk`
  compares both cases with `ignore` on sample roots, patterns and paths.
- `RuleFilePath`, `DiagnosisProbes` and `walk::entry_kind` report the
  filesystem facts they name, stably for one decision (the later read is not
  covered); the Git backend's existence walk and candidate lists are right.
- `str::lines` splits the file, vstd's specification of
  `char::is_whitespace` matches std's, and the `chars()` conversions and
  range-to-`String` rebuilds are faithful; sample tests compare both grammars
  with the replaced code. Provenance assumes `ignore` reports its
  highest-index matching glob and keeps the recorded text as
  `Glob::original`; when no recorded text equals it (a line ending in an
  escaped space, per the hand-written `ignore_original_text` model),
  `glob_line_info` reports line 0, preserved as-is.
- `path_match_kind` and `ExclusionLevels::query_level` map `ignore::Match` to
  `MatchKind` faithfully; the `policy::vocab` `From` conversions in
  `src/config.rs` are faithful; `effective_case_insensitive` passes
  `cfg!(any(target_os = "macos", windows))` as the platform-may-alias fact.
  No exec function called here has a `requires` clause; the oracles' indexing
  relies on bounds the kernels prove at their own calls.

**Still unverified here**
- Glob compilation and matching, the handling of lines or files `ignore`
  rejects, and the exclusion matcher's pattern order
  (`build_excluder_with_case` adds the built-ins, then the extras):
  `ignore`/`globset` semantics, outside Verus's reach here, and
  configuration glue.
- The applicable-directory, ancestor and exclusion-level lists: `std::path`
  logic; could move into a kernel over the `waft_verified::path::model`
  components.
- Rule-file reads, gate probes and the wt-0.39 walk, including resolving a
  target against its rule file's directory (`RepoRelPath::normalize`, see
  Repository-relative paths): I/O glue.
- The wt-0.39 subtraction (`retain` against a `HashSet<String>`): vstd has no
  `Vec::retain` specification, and a membership-mask kernel would add little.
- `eligible_records`' composition (dedup, exclusion, Git-ignored and
  regular-file filters, when to diagnose): sequencing over I/O answers; could
  move into a kernel.
- Rendering (`status_from_verdict`, `format_worktreeinclude_status`,
  `note_empty_selection`), `engine_for` dispatch and the
  `TOOLING_V1_PATTERNS` list: formatting, dispatch and data.

**Size:** 922 spec / 883 proof / 366 exec lines, 0 trusted. `line_count`
does not mark the oracle contracts trusted: it counts their spec functions as
spec and their method signatures and contract clauses mostly as exec. By
file: `rules/precedence.rs` 316/354/133, `rules/reference.rs` 209/261/0,
`rules/lines.rs` 126/115/65, `exclusion.rs` 74/101/28, `selection.rs`
197/52/140; `rules.rs` holds only documentation and module declarations.

### Planner

**Production code:** `src/planner.rs`, `src/eligibility_groups.rs`, and the
plan types in `src/model.rs`. **Kernels:**
`waft_verified::planner::{destination, entry}`, `waft_verified::groups`, and
`waft_verified::tally`, which builds on `waft_verified::execute`.

For each manifest path, two step kernels choose the next read-only query and
interpret its answer, ending in the path's plan entry. `groups` orders and
deduplicates the manifest and decides directory promotion. `tally` counts
dry-run entries and computes the dry-run exit status with the executor
kernel's own failure decision.

**Verified decisions**
- Destination state and query order: `destination::classify_step` in
  `classify_destination_state`. It is reached from `plan_entry`, and from the
  `list` and `info` subcommands through `classify_destination`.
- Each path's plan entry and its repair-or-replace guard:
  `entry::plan_entry_step` (which calls `entry::same_content`) in
  `plan_entry`.
- `groups::sort_unique` in `plan_with_overwrite_support` and in
  `EligibilityGroups::from_files`, which only tests and benchmarks call.
  `groups::dir_copyable` in `WalkContext::walk_dir`.
  `groups::{outermost, uncovered}` in `eligibility_groups::compute`.
- `tally::{tally, shows_entry, dry_run_summary}` in `render_dry_run`.
  `tally::planning_failures` in `planner::planning_failures`, which decides
  the dry run's exit status.

**What the proofs establish**
- Let `w` range over the complete sets of answers (`DestWorld`, `EntryWorld`)
  that agree with what has been observed. A query named by `classify_step`
  is, for every such `w`, the first unanswered query in the short-circuit
  order `dest_queries(w)`. An answer it gives equals `classify_spec(w)` for
  every such `w`, so no skipped query could change it. `plan_entry_step`
  meets the same contract for `entry_queries` and for `plan_entry_spec`,
  which fixes the plan's kind without its payloads; a query hands the
  observation back unchanged. Its plan carries exactly the observed payloads:
  the source snapshot, the destination snapshot for a replacement or repair,
  or the error of the failed query. *Postconditions (`step_sound`,
  `entry_step_sound`).* `theorem_driver_follows_spec`: for an observation
  that agrees with `w` and has answered exactly the first `k` spec-order
  queries, the next step asks query `k`, or returns the spec's answer once
  all are answered. `lemma_some_world_agrees`: the contracts are not vacuous.
  *Lemmas about the contracts.*
- `classify_spec` checks, in order: tracked, symlinked parent, missing,
  symlink or not a regular file, then the comparison, where it mirrors the
  existing rule that a comparison error is an untracked conflict. A tracked
  path needs no query. A comparison error never yields "up to date" or
  "permissions differ". `plan_entry_spec` never plans a write to a
  destination classified as tracked, and snapshots neither file for one,
  whatever the flags. Without `--overwrite`, a copy only goes to a missing
  destination. Replacing or repairing requires `--overwrite`, platform
  support, a regular source, an untracked or permission conflict, and both
  snapshots. A repair also requires a permission conflict and equal pinned
  `(len, fingerprint)` keys; with unequal keys it is a replacement. Where the
  platform cannot replace, nothing existing is written, and an untracked or
  permission conflict under `--overwrite` is a failure. A source that cannot
  be examined is a failure, never a skip. The destination is snapshotted only
  after the source snapshot succeeds. `theorem_end_to_end_*` restate the
  tracked, copy-to-missing and replace-or-repair cases in terms of the
  destination's raw answers, through `classify_spec`. *Lemmas about the
  specifications.*
- After `sort_unique`, the keys are strictly increasing in byte order, so no
  path appears twice. The key set is unchanged, and every element is one of
  the originals. *Postcondition.* Distinct byte strings can still name one file
  on a case- or normalization-insensitive destination, as with the `sort` and
  `dedup` it replaced.
- `dir_copyable` holds iff the directory has at least one entry and every
  entry is an eligible regular file or a directory the walk reported
  copyable. `outermost` keeps an entry iff no listed directory is a proper
  `/`-ancestor of it. `uncovered` flags exactly the eligible paths that are
  not in the covered list. Neither needs sorted input. *Postconditions.* No
  kept directory is a `/`-ancestor of another, distinct kept paths have no
  common descendant, and every listed directory is, or lies under, a kept
  directory, all such kept directories having the same path
  (`theorem_outermost_antichain`, `theorem_outermost_disjoint`,
  `theorem_outermost_covers`). If every covered path is eligible, the
  eligible paths are exactly the covered ones plus the flagged ones, with no
  overlap (`theorem_partition`). *Lemmas over those postconditions.*
- `tally` counts each entry exactly once, under its kind. `planning_failures`
  is `None` iff nothing failed, and otherwise `(failures, failures + copies of
  all three kinds)`. `shows_entry` shows every entry without `--quiet` and
  only failures with it. `dry_run_summary` returns no summary under
  `--quiet` and otherwise shows a clause iff its counter is non-zero.
  *Postconditions.* By `planning_failures`'s last postcondition and
  `theorem_run_plan_tally_is_executor_counts`, it returns the value that
  `execute::failure_summary`'s postcondition fixes for every tally
  `execute::run_plan` can return from a dry run of a plan with the same entry
  kinds. `executor::report_has_failures` applies the same `failure_summary`
  to a real run. By `theorem_real_run_fails_if_dry_run_fails`, if the dry run
  fails, every tally `run_plan` can return from a real run of that plan, for
  any publication results and with or without a destination root, has a
  failure, the same total, and at least as many failures. *Lemmas over the
  executor kernel's own specification, not over a separate model.*

**Assumed, not proved**
- The loops in `plan_entry` and `classify_destination_state` perform each
  named query on the right path and store the answer in that query's field;
  unit tests with a recording filesystem check the order on sample paths.
  `EntryObs` and `DestObs` are plain structs, not typestates. `FileSystem`
  answers are trusted. On `RealFs`, `exists` follows symlinks, so a dangling
  destination symlink is observed as absent and classified `Missing`; the
  kernel mirrors this existing behavior.
- Trackedness comes from `GitBackend::tracked_paths`. Platform support is
  `overwrite_supported` in `src/fs.rs`, i.e. `cfg!(unix)`. Unit tests pin the
  exhaustive conversion matches `compare_obs`, `source_kind_obs`,
  `kernel_snapshot` (with `FileSnapshot::content_key`), `destination_state`,
  `skip_reason` and `entry_kind`, and the agreement of `entry_kind` with
  `executor::kernel_entry`; `planned_entry` is exercised only through
  plan-level tests. Two snapshots count as having the same content when
  their lengths and 64-bit FNV-1a fingerprints match, which is not byte
  equality. The real-run theorem is about one
  plan, not two runs over a changing tree.
- The walk reports each `ChildKind` faithfully and applies `dir_copyable`
  bottom-up. `compute` meets `theorem_partition`'s precondition by
  construction, but nothing checks it. `PathKey::key_bytes` on `RepoRelPath`
  returns the same bytes on every call. `std`'s `sort_by` meets vstd's
  specification. No exec function that production calls here has a
  `requires` clause.

**Still unverified here**
- The query loops and the queries: I/O glue. With more work, the loops could
  become verified drivers.
- The list handling in `plan_with_overwrite_support`: the empty plan when there
  is no destination root, the flattening of promoted directories, and the
  final `sort_by`. This could move into a kernel. Until it does, the claim
  that promotion neither adds nor drops a planned file depends on this glue
  and on `compute`'s.
- The source walk, its recursion, and `compute`'s zipping of the kernel flags:
  I/O and collection glue. The `groups` docs argue that a directory's
  copyability reflects its whole subtree, but nothing proves it; a proof
  would need a tree model.
- Messages, dry-run text and output channels: formatting that Verus does not
  model here. `SkipReason::describe` and the small `model.rs` projections
  could move into a kernel.

**Size:** 929 spec / 1006 proof / 646 exec lines, 0 trusted. By file:
`planner/entry.rs` 369/309/287, `planner/destination.rs` 180/117/109,
`groups.rs` 163/428/176, `tally.rs` 217/152/74. `planner.rs` has no spec,
proof or exec lines (module declarations and documentation only).

### Executor and Git index lock

**Production code:** `src/executor.rs`: `execute` with `PlanRunner`,
`report_has_failures`, `render_report`, `GitIndexLock` with `LockAttempts`,
and the Unix `interrupt_cleanup` module. **Kernels:**
`waft_verified::execute`, `waft_verified::index_lock`, and the ghost model
`waft_verified::index_lock::model` (no executable code).

The kernels decide which plan entries are published, how each is counted and
reported, the failure status and its `(failed, total)`, which rows and
summary clauses print with and without `--quiet`, how the destination
`index.lock` is taken and, on Unix, the order of every step of its
`SIGINT`/`SIGTERM` cleanup protocol, release included. The signal-safety
properties P1–P7 (there is no P5 or P6) are proved about a hand-written model
of that protocol, not about the drivers.

**Verified decisions**
- Publication, counter and row of each plan entry: `execute::run_plan`, in
  `execute`, with `PlanRunner` as the `EntryRunner`. The CLI reaches only its
  real-run route with a destination root: `copy --dry-run` returns before
  `execute` (the dry-run route is what the [Planner](#planner) section's
  `tally` theorems compare against), and planning without a root yields no entries.
- Exit status: `execute::failure_summary`, in `report_has_failures`. Printed
  rows and summary clauses: `execute::{prints_row, summary_plan}`, in
  `render_report`.
- Lock acquisition, on every platform: `index_lock::acquire`, in
  `GitIndexLock::acquire`, with `LockAttempts` as `AcquireOps` and
  `INDEX_LOCK_RETRY_DELAYS.len()` (2) retries.
- Unix signal-time cleanup, in `interrupt_cleanup`: `index_lock::stage` in
  `stage`, `arm` and `disarm` in their namesakes, each reached from the
  `LockAttempts` method of that name; `on_signal` (which runs `claim_armed`)
  in the handler `handle_interrupt`; `resume_pending` in
  `resume_pending_signal`, reached from every `LockAttempts::check_deferred`;
  `release` (which also runs `resume_pending`) in `release`, from
  `Drop for GitIndexLock`.

**What the proofs establish**
- For some publication results, one per entry, the runner's log grows by
  exactly `plan_log`, each entry's events in plan order. Each counter counts
  the entries of its `entry_counter` class (a copy takes its expected class in
  a dry run, fails without a root, else takes its publication's class, any
  error failing); the six counters sum to the plan length, and the four
  behind the exit status sum within `usize`. *Postcondition of `run_plan`.*
- On that log: exactly the `Copy` entries are published, each once, in plan
  order (none in a dry run or without a root); the rows name exactly the
  entries not up to date, each once, in order; each publication is followed
  at once by its entry's row, carrying its result or its very error; each
  class but up-to-date has as many rows as its counter, and none is up to
  date; `failed > 0` iff some row is a failure. A dry-run row and class are
  those of the matching successful publication. *Lemmas about the
  specification that postcondition refers to.*
- `failure_summary` is `Some` exactly when `failed > 0`, then
  `(failed, copied + replaced + permissions_repaired + failed)` with
  `1 <= failed <= total`. `prints_row` prints every failure, and every other
  row exactly when not quiet; `summary_plan` prints exactly when not quiet,
  each optional clause exactly when its counter is non-zero.
  *Postconditions.* That the `FAILED:` lines number `CopyReport::failed`
  also needs the row glue below, so it is not a theorem.
- `acquire` stages, makes at most `retries + 1` `create_new` attempts, and
  retries only contention. Each attempt is followed at once by a
  deferred-signal check told whether it created the file; delay indexes stay
  below `retries`, so `INDEX_LOCK_RETRY_DELAYS[index]` is in bounds. If that
  check lets it go on, success arms and checks again, and giving up or another
  error disarms and checks again; "after N attempt(s)" counts the attempts.
  It returns the error of a last check that acted on a deferred signal, else
  the created file, the error of an attempt that failed other than by
  contention, or the "locked" error; it is `Ok` exactly when it armed and the
  final check passed. *Postcondition of `acquire`;
  `lemma_acquire_bounded_and_checked`, a lemma about `acquire_log`.*
- Each signal-protocol driver issues exactly the effects its `*_events`
  specification gives for what its loads, swaps and compare-exchanges saw.
  Staging resets the descriptor and writes the path strictly between the
  stores of `WRITING` and `STAGED` (a path of 4096 bytes or more only stores
  `EMPTY`). Below, a removal is a call of the identity-checked
  `remove_recorded_lock`. The handler attempts removal only right after its
  claim `ARMED -> EMPTY` succeeded, and returns without re-raising exactly
  when both loads saw `STAGED`. A resume attempts removal only if it took a
  signal, the caller holds the lock and the state was not `EMPTY`, right
  after storing `STAGED`; each re-raise directly follows a disarm. Release
  does nothing more unless its claim `ARMED -> STAGED` succeeds, then attempts
  removal once and disarms before taking a deferred signal. *Postconditions
  of the drivers; lemmas about the `*_events` specifications.*
- `index_lock::model` interleaves main-thread steps (one per atomic operation
  or system call of `GitIndexLock::acquire` and its `Drop`, plus `Holding` for
  publication; each `Pc` variant names the call it mirrors), complete handler
  runs at any step boundary, and other Git writers (and, unless `coop`,
  someone renaming or deleting our lock), with `SIG_DFL`, `SIG_IGN` or a
  returning handler as previous disposition. For every finite interleaving
  from a state with none of our locks on disk: P1, no removal uses a stale
  descriptor or path, and no main-thread or handler step changes another
  writer's lock; P2, an acquisition with its release attempts removal at most
  once unless the previous handler returns, and a resume never decides to
  remove after a removal; P3, with a path that fits the buffer, our lock is
  never on disk once a signal has killed the process or while no acquisition
  is in progress, and none is left behind; P4, in a live process a deferred
  signal always has a consuming check ahead, except one recorded inside a
  resume between its take and its disarm, which outlives it only under a
  returning handler; P7, with a path that fits, nobody else renaming or
  deleting our lock, and a non-returning previous disposition, publication
  happens only while our lock is on disk, and while it is held it is armed
  with nothing deferred. `example_*` lemmas show the P2, P3 and P4 exceptions
  occur in the model. P7's clause on `Holding` states what a successful
  acquisition leaves, and P3's on `Idle` what release or a failed acquisition
  leaves.
  *Lemmas about a hand-written model; nothing mechanically ties its steps to
  the drivers or to `executor.rs` (review does). It shares only the kernel's
  constants and its `spec_defers` and `spec_resume_removes` predicates; its
  theorems are stated through `spec_defers`, so they do not pin deferral to
  `STAGED` alone.*

**Assumed, not proved**
- The oracles do what their names say; their `ensures` only log the call.
  `PlanRunner::publish` attempts one publication and pushes no row; each
  `report_*` pushes one `CopyResult` saying what its row says. Each `Cells`
  and `Stager` method is the one operation its name says: a `SeqCst` access to
  `STATE`, `PENDING_SIGNAL` or `LOCK_FD`, the copy into `PATH` of the bytes
  whose length `stage` checked, handler installation, the identity-checked
  unlink, or restore-and-raise. On Unix `LockAttempts::{stage, arm, disarm,
  check_deferred}` run the drivers above, `check_deferred` failing exactly
  when `resume_pending` returned a signal; `try_create` records the new
  file's descriptor and treats `AlreadyExists`, and only it, as contention.
  `FileSystem::copy_file` calls `before_publish` at most once, before
  publishing.
- `kernel_entry`, `kernel_expectation`, `kernel_publish_kind` and
  `row_counter` map each variant to its namesake (exhaustive matches; the last
  three pinned by `kernel_conversions_are_one_to_one`, `kernel_entry`
  exercised by `execute_counts_each_entry_once_and_reports_all_but_up_to_date`).
  `failure_summary`'s requirement holds for every report `execute` builds (a
  `run_plan` postcondition, not checked at run time); a hand-built
  `CopyReport` passed to the public `report_has_failures` gets plain `usize`
  addition, as before.
- The model's hypotheses: waft is single-threaded; `SeqCst` gives one order;
  the handler runs to completion while the main thread waits (its `sa_mask`
  blocks both signals), so its two loads agree and the model omits
  `on_signal`'s withdraw branch; `raise` under `SIG_DFL` terminates; one lock
  path; handler installation is not a separate step; `(st_dev, st_ino)`
  identifies the open lock file; and `remove_recorded_lock`'s `fstat`/`lstat`
  identity check and `unlink` act as one step, which POSIX cannot provide.
- After `exec` a process has `SIG_DFL` or `SIG_IGN` for these signals, and
  waft installs no other handler for them, so the CLI meets P2's and P7's
  disposition hypothesis and P4's exception cannot arise, unless a program
  using the library installed a returning handler first. P3 and P7 also need
  the lock path to be under 4096 bytes (`PATH_CAPACITY`).
- `on_signal`, `claim_armed` and the `Cells` methods they call run inside the
  signal handler; that their compiled code does not allocate, lock or panic
  is by inspection (generic, statically dispatched, ghost code erased). Verus
  does not check async-signal safety.
- Unix only. Elsewhere `interrupt_cleanup` does nothing and `acquire` never
  sees a deferred signal; no kernel or model covers lock release there.

**Still unverified here**
- In `PlanRunner::publish`: the index-path lookup, taking the lock and
  rechecking trackedness inside `before_publish`, calling `copy_file`, and
  dropping the lock only after it returns. Why: I/O glue whose order a kernel
  could drive with more work; holding the lock across `copy_file` rests on
  drop order and the `copy_file` contract. Tests pin the order; on Unix the
  publication typestates take the callback's result as a check before any
  publication or `fchmod`; the model's `Holding` step stands for the span.
- `CopyResult` rows, their messages and the report text (`report_*`,
  `render_report`). Why: production types and `format!`, beyond Verus here;
  each mapping is an exhaustive match.
- `record_file` in `try_create`, closing the file after `release` in `Drop`,
  and the error `resume_deferred_signal` builds. Why: I/O glue; the model's
  `Record` and `Close` steps stand for the first two.
- `create_new`, the retry sleep, `fstat`, `lstat`, `unlink`, `sigaction` and
  `raise`; handler installation (`install_unless_ignored`, which leaves an
  inherited `SIG_IGN` alone); the disposition `restore_previous_and_raise`
  puts back. Why: system calls and FFI that Verus does not model.

**Size:** 1368 spec / 968 proof / 423 exec lines, none counted as both, and 0
trusted. By file: `execute.rs` 426/427/194, `index_lock.rs` 339/341/229,
`index_lock/model.rs` 603/200/0.

### File checks: fingerprints, path components, comparison and stability

**Production code:** `src/fs.rs`, apart from the anchored-publication
protocol. **Kernels:** `waft_verified::{fingerprint, components}` and
`waft_verified::fs_checks::{compare, errno, stability, steps}`.

Production makes every syscall and passes the results to these kernels as
plain values; no kernel here takes an oracle. Each exec function is proved to
return its specification, which transcribes the code it replaced (for
fingerprints, the standard FNV-1a fold), and lemmas restate several
specifications in terms of bytes or inode identities.

**Verified decisions**
- Fingerprints: `fingerprint::Fingerprinter` in `fingerprint_reader`, behind
  every real-filesystem snapshot; `fingerprint::fingerprint` in
  `fingerprint_bytes`. Walk names: `components::{relative_components,
  absolute_walk}` in `relative_components` and `open_canonical_directory`.
- Comparison: `compare::{StreamComparison, MatchedContents}` in
  `RealFs::compare_files`; `compare::compare_contents` in the default
  `FileSystem::compare_files`. `RealFs` overrides that method and
  `FileSystem::file_snapshot` (the only user of `fingerprint_bytes`), so the
  defaults run only for test filesystems. Stability:
  `stability::{opened_stably, unchanged_through_path, unchanged_handle}` in
  `open_stable_regular_file`, `ensure_open_file_unchanged` and
  `ensure_open_handle_unchanged`.
- `errno::is_listed` in `rename_flag_unsupported`; `steps::{attempts_reflink,
  TempNameAttempts, creates_missing_directory, after_mkdir}` in
  `attempts_reflink`, `create_anchored_temp` and `open_relative_parent`.

**What the proofs establish**
- `fingerprint` returns the FNV-1a 64-bit fold of its input, and
  `Fingerprinter::finish` returns the fold of the concatenation of every chunk
  passed to `update` since `new`, whatever the chunk sizes. *Postconditions,
  with the `Fingerprinter` type invariant.* The fold matches the published
  test vectors for `a` and `foobar`, and inputs of equal length that differ
  in exactly one byte get different fingerprints. *Lemmas about the spec.*
- Both walks return exactly their spec: the former loop, run over
  `std_components`, a hand-written model of Unix `Path::components`.
  *Postconditions.* In bytes: a relative path is refused as unsafe iff it
  starts with `/`, is `.`, starts with `./`, or has a `..` segment; otherwise
  as empty iff every segment is empty or `.`; otherwise it yields its other
  segments, in order, and at least one. The absolute walk returns `None`
  without a leading `/`; otherwise its names are the segments that are
  neither empty nor `.` before the first `..`, and it is marked refused iff
  some segment is `..`. Every name either walk returns is non-empty, has no
  `/`, and is not `.` or `..`. *Lemmas about the spec, stated in bytes and so
  independent of the model.* The kernel mirrors the existing behavior,
  including reading `a//b`, `a/./b` and `a/b/` as `[a, b]`.
- `begin` returns `None` (production: `ContentDiffers`) iff the recorded
  lengths differ; `compare_chunks` returns `Differ` iff two chunks differ in
  length or bytes. The type invariants, private fields and absence of `Clone`
  and `Copy` mean `verdict` is reached only after equal recorded lengths and
  identical chunk pairs ending in two empty chunks. `verdict` is `Equal` iff
  the permission signatures match, else `PermissionsDiffer`. The kernel
  mirrors the existing behavior, including treating chunks of different sizes
  as different even when one is a prefix of the other. `compare_contents` is
  `Equal` iff the buffers are equal, else `ContentDiffers`. *Postconditions
  and type invariants.*
- `opened_stably` holds iff the path after the open is a regular file and is
  the same file as the handle, which is the same file as the path before.
  `unchanged_through_path` adds that the initial, handle and path
  observations agree in length, mtime, ctime and permission signature;
  `unchanged_handle` asks the same of the handle alone, against the initial
  observation. "Same file" means the same `(dev, ino)`, or the same state
  where either observation has no identity. *Postconditions.* With inode
  identities, a predicate holds only if all its observations name one inode.
  *Lemmas about the spec.*
- `is_listed` is table membership. `attempts_reflink` holds iff the strategy
  is `CowCopy`, or `Auto` where the platform prefers reflinks. `try_another`
  grants a call iff fewer than 128 calls on that counter came before.
  `creates_missing_directory` holds iff `create` holds and `openat` reported
  `ENOENT`. `after_mkdir` maps `Created`, `AlreadyExists` and `Failed(e)` to
  `Ok(true)`, `Ok(false)` and `Err(e)`. *Postconditions, with the
  `TempNameAttempts` type invariant.*

**Assumed, not proved**
- `read()` returns successive bytes and 0 only at end of file, and production
  passes each `&buffer[..count]` in order. The ghost streams and typestate
  record only what was passed in. `begin` gets the opened handles' metadata.
- Unchanged metadata implies unchanged bytes only if the OS updates `ctime`
  on every write and keeps `(dev, ino)` fixed while the file is open. No
  kernel compares the metadata length with the bytes consumed. FNV-1a is
  64-bit and not cryptographic.
- `std_components` matches Unix `Path::components` and an `OsStr` is its
  bytes (tested on all strings of up to 7 characters over `a . / \` and up to
  4 over `é . /`). `observation` and `modified_stamp` encode metadata
  faithfully and injectively (unit-tested only). Enum and errno conversions
  are exact, the reflink preference is `cfg!(target_os = "macos")`, and the
  four "unsupported" errnos are a specification choice.
- vstd's slice specs (indexing, sub-slicing, equality) are trusted, including
  the `external_body` `slice_to_vec` that `components` runs. `components`,
  `errno`, `TempNameAttempts` and the mkdir steps run only on Unix.

**Still unverified here**
- Read loops, the order of re-checks, and every `stat`, `open`, `openat`,
  `mkdirat` and `fsync` call: I/O glue. The 128-create bound and the
  unreachable `expect` in `open_relative_parent` follow from kernel
  postconditions and lemmas only by informal argument.
- The metadata accessors behind `observation`: vstd has no specs for
  `fs::Metadata` or `SystemTime`. `FileSnapshot` assembly, the `(dev, ino)`
  check in `ensure_same_relative_parent` and the staging-name encoding: these
  could become kernels with more work.
- `try_reflink_anchored` (its macOS loop keeps its own 128-name limit), the
  `FileSystem` probes and the non-Unix `copy_file_path_fallback`: I/O glue,
  or crates that Verus does not model.

**Size:** 664 spec / 693 proof / 311 exec lines, plus 10 counted as both proof
and exec, and 0 trusted. By module: `fingerprint` 75/113/36, `components`
347/488/69, `fs_checks` 242/92/206.

### Publication protocols

**Production code:** `src/fs.rs`, Unix builds only: `copy_file_anchored_unix`,
`AnchoredTempGuard`, `remove_proven_name`, `publish_noreplace`,
`replace_verified_destination`, `undo_lost_exchange`,
`replace_by_displacement`, the repair and error-mapping helpers, and the probe
and system-call wrappers. **Kernels:** `waft_verified::publish::{ordering,
removal, noreplace, replace, repair, trace, model}`.

On Unix, once the source and any existing destination have matched their
plan, typestate step machines decide each step of the anchored publication:
check order, which inode a staging or displaced name may be unlinked for,
no-clobber publication with its link fallback, replacement by exchange (with
undo) or displacement (with restore), and permission repair. Production runs
the probe or call a state names and feeds back the answer; each state keeps a
ghost trace of what it asked and was told. Verus proves properties of those
traces and, separately, theorems about a hand-written filesystem model; the
latter are about the model.

**Verified decisions**
- Check order and the `Ready` token both publication protocols consume:
  `ordering::Staged::created` (called by `AnchoredTempGuard::new`; it also
  creates the staging guard state) and the `StagingCheck<N>` steps, in
  `copy_file_anchored_unix`.
- Guard entitlement: `removal::GuardState` (created only by `Staged::created`
  and `replace::Aside::after_rename_aside`), in `AnchoredTempGuard` and its
  `Drop`. Probe, then unlink only on proof: `removal::ProvenRemoval` and its
  steps, in `remove_proven_name`, which runs every removal a kernel hands out.
- No-clobber publication (`RENAME_NOREPLACE`; only if that is unsupported, a
  hard link, then a proven removal of the extra name): `noreplace::NoReplace`
  and its steps, in `publish_noreplace`, for a missing destination and for
  the displacement path's publication and restores.
- Exchange, undo and displacement: `replace::{Exchange, SwapProbe,
  SwapPinned}`, the `Undo*` states, `Reprove`, `Aside`, `Lstat`,
  `DisplacedPinned`, the continuations their removals and publications report
  back to, and the verdict enums, in `replace_verified_destination`,
  `undo_lost_exchange`, `replace_by_displacement`, `undo_error` and
  `Displaced::result`.
- Permission repair: `repair::Repair`, `RepairCheck<N>`, `RepairChmod`,
  `RepairReadBack` (`repair_verdict`), `RepairRevert`, `RepairProveName` and
  `RepairSync`, in `copy_file_anchored_unix` and `revert_permission_repair`.

**What the proofs establish**
- Each check step is `Ok` iff the check result fed to it was, passes errors
  through unchanged, and appends exactly its own event, so every `Ready`
  records the six checks in order (source unchanged, staging mode, sync,
  `before_publish`, parent revalidation, staging-name proof).
  *Postconditions; type invariant of `Ready`.* That steps cannot be skipped
  or reordered, and that both publication protocols consume a `Ready`, is
  Rust typing (consuming typestates, private fields, no `Clone`), not Verus.
- A proven removal ends `Removed` iff its trace is a probe that found the
  entitled identity under its name followed by a successful unlink of that
  name; without an identity it issues nothing. *Type invariant
  (`removal_trace`) of `RemovalDone`.* A guard state's name and identity
  never change, it is never re-armed, and `drop_removal` hands out at most one
  removal, entitled to exactly that name and identity. *Postconditions of
  `GuardState`'s methods and of every kernel step taking `&mut GuardState`.*
- No-clobber (NR1–NR4): the only unlink is of the published-from name, right
  after this publication's own successful link and a probe for the identity
  it was given, and there is none without an identity; a link only follows
  an unsupported `RENAME_NOREPLACE`; the result is `Err` iff nothing was
  published, `Ok(Consumed)` iff the source name is gone; for a missing
  destination `into_result` disarms the staging guard when the result is `Ok`
  and leaves it unchanged otherwise. *Type invariant (`noreplace_trace`) of
  `NoReplaceDone`, the lemma `theorem_noreplace` about it, and
  `into_result`'s postcondition.*
- Trace safety (`trace::safe`): every unlink immediately follows a probe
  proving the same name holds an inode the run is `entitled` to remove there
  (its own staging file, under the staging name; whatever the name holds when
  the event before the probe is the run's own successful link from it, an
  inode this clause does not pin down; or the verified planned destination
  `v`, after it was moved under that name, found there and the pinned
  descriptor matched), so the destination is never unlinked; every link
  immediately follows an unsupported `RENAME_NOREPLACE` of the same name;
  every exchange immediately follows a proof that the destination holds `v`
  with its planned snapshot (or, going back, the run's prepared file); a move
  aside immediately follows an unsupported exchange and a fresh re-proof.
  *Type invariant of every replacement state and of `ExchangeDone`,
  `UndoDone` and `DisplaceDone`; for no-clobber traces, the lemma
  `lemma_safe_append_noreplace`.*
- Verdicts: exchange `Ok` means the staging name was then proved to hold
  `v`, the pinned descriptor matched, and the trace ends with a proven unlink
  of `v` from the staging name; every undo path ends with the staging guard
  disarmed; each displacement verdict says whether the replacement was
  published, whether the staging name is gone after a publication, and
  whether the displaced name is gone (`names_left`), and carries exactly the
  errors and leftover reasons that produced it; `Replaced` means the moved
  entry was identified as `v` and matched, the replacement published, and
  the trace ends with a proven unlink of `v` from the displaced name. *Type
  invariants of `ExchangeDone`, `UndoDone`, `DisplaceDone`; step
  postconditions.*
- Repair (R1–R5): it starts iff the planned destination and source snapshots
  agree on length and fingerprint; the `fchmod` mode is the planned source's
  full `st_mode`, handed out only after its five checks (source unchanged,
  `before_publish`, parent revalidation, destination-name proof, pinned
  read-back); `repair_verdict` judges content first (the kernel mirrors the
  existing behavior, including counting a failed read-back as changed
  content); a second `fchmod` is handed out only for changed content, with
  mode `expected.permissions & 0o7777` (the planned destination's permission
  bits); `Repaired` exists only with the trace (checks, `fchmod`, intact
  read-back, name proof, `fsync`), which changes no name. *Postconditions;
  type invariant of `Repaired`; two lemmas on the repair traces.*
- The model (`publish::model`): `Fs` maps `Dest`, `Staging` and `Displaced`,
  under one directory handle, to inodes `(st_dev, st_ino)`; a missing key is
  a vacant name. `effect` is each event's namespace change; `consistent` says
  an answer was possible ("holds" saw that inode; a successful rename or link
  found the destination vacant; "does not hold" and failures constrain
  nothing). Between waft's steps the environment may change the names
  arbitrarily, subject only to the axioms A-window (`window_quiet`: nothing
  changes between a successful probe and an unlink right after it), A-fresh
  (`aside_fresh`: the displaced name is vacant at every move aside) and, for
  T1′ only, A-link (`link_window_quiet`: nothing changes between a successful
  link and the next step).
- On `safe` traces with consistent answers: T1 (`theorem_no_foreign_loss`,
  with A-window and A-fresh): an inode a step makes vanish from all three
  names was proved by the probe just before, under the name unlinked, and the
  run was `entitled` to remove it there; T1′
  (`theorem_only_own_or_verified_lost`, plus A-link): it is the run's own
  staging file or `v`; T2 (`theorem_dest_never_clobbered`, no axioms): what
  the destination holds before a step is under some modeled name after it;
  NR5 (`theorem_noreplace_keeps_dest`, no axioms): in a trace without
  exchange or move aside, no step changes what an occupied destination holds.
  T3 (`theorem_repair_keeps_namespace`): a trace without namespace events
  changes no name, which follows from `effect` alone. `lemma_model_witness`
  shows T1's hypotheses are satisfiable by an exchange trace in which `v`
  vanishes. *Lemmas about a hand-written model: nothing mechanically ties
  `effect`, `consistent` or the axioms to the operating system; production is
  linked only through the traces the kernels record.*

**Assumed, not proved**
- Production issues exactly the probe or call each state names, at that
  point, on the names and identities it hands out, and feeds back the true
  answer (an incomplete probe as "does not hold"). No kernel function it
  calls has a `requires` clause.
- `Exchange::after_destination_verified` and `Reprove::reproved` record the
  destination proof without receiving it; production calls them only after
  `ensure_name_refers_to_file` and `ensure_open_snapshot_matches` succeed,
  with identities from `fstat` of the pinned and staging descriptors. It
  calls `before_publish` once.
- `Ident::same_inode`, which the kernel calls to decide whether the displaced
  entry is `v`, compares `(st_dev, st_ino)` as the probes do; `flag_attempt`
  and `repair_snap` convert faithfully (unit tests in `kernel_glue_tests`);
  the errno table of `rename_flag_unsupported` (`ENOTSUP`, `EOPNOTSUPP`,
  `EINVAL`, `ENOSYS`) is what "flag unsupported" means. The staging guard is
  built from `fstat` of the file just created with `O_EXCL` (on macOS, of the
  `fclonefileat` clone reopened by name); each guard state lives for the whole
  publication and is never replaced, and production calls
  `GuardState::disarm` only in tests.
- "Reads back as planned" is `FileSnapshot` equality (length, 64-bit FNV-1a
  fingerprint, mode, identity); `same_content` compares length and
  fingerprint. Neither is byte equality. `fchmod` applies exactly the
  requested bits; where the OS clears setgid or a FAT-like mount cannot store
  the mode, the read-back does not show the source's mode and `Repaired` is
  not reached.
- A guard's `Drop` runs on early return and unwinding (Verus does not model
  unwinding). Drop removals meet the removal contract, but their outcome is discarded and
  they are in no trace the model theorems take; the ordering chain, each
  publication record and each drop removal are separate traces, and no
  theorem composes them into one run.
- `effect` and `consistent` state the POSIX semantics of `renameat2`
  (`RENAME_NOREPLACE`, `RENAME_EXCHANGE`; `renameatx_np` on macOS),
  `renameat`, `linkat` and `unlinkat` that waft relies on. A-window is the
  gap POSIX cannot close (it has no conditional unlink); A-fresh rests on the
  move aside being a plain `renameat` onto a fresh 128-bit random name
  (`next_displaced_name`). The model tracks which inode each name holds, not
  contents.
- Unix only: elsewhere production refuses to replace or repair an existing
  destination and publishes a missing one with `copy_file_path_fallback`,
  which uses no publication kernel.

**Still unverified here**
- The probes and system calls, including the final `fsync` of the
  destination directory, and the test seams (no-ops outside tests). Why: I/O
  glue.
- Work outside every trace: matching the source to its plan, pinning the
  destination (`open_verified_destination`), and allocating and filling the
  staging file (`create_anchored_temp`; `try_reflink_anchored`, including its
  own unlink of a clone that failed). Why: I/O glue; these calls are in no
  trace, so `safe` and T1 do not cover them.
- The two input-less evidence steps above, and the identity comparisons
  (`same_inode`, `ensure_name_refers_to_file`). Why: could move into a
  kernel; the steps could take the check results as inputs, as the ordering
  steps do.
- Mapping verdicts to `io::Error` kinds and messages (`Displaced::result`,
  `undo_error`, `publication_result`, `AnchoredTempGuard::left_error`,
  `revert_permission_repair`). Why: `io::Error` and `format!` are beyond
  Verus here.
- The end of a repair: the revert `fchmod`'s result reaches no state, and
  `ModeChanged` and `NameTaken` have no finished record. Why: could move into
  the kernel with more work.
- What the destination holds when production reports `Created` or
  `Replaced`: the proofs cover what was moved, swapped out or removed. Why:
  it would need a probe after publication, changing the call sequence.

**Size:** 1520 spec / 1054 proof / 1003 exec lines, plus 42 counted as both
proof and exec, and 0 trusted. By file: `replace` 532/415/482, `repair`
280/124/213, `trace` 188/133/50, `model` 158/208/3, `noreplace` 152/111/90,
`removal` 141/32/115, `ordering` 69/31/50; `publish.rs` is documentation.

## Trust boundary

`--no-cheating` and the zero "Trusted" count show that `waft-verified` has no
unproved step of its own. The guarantees above still rest on the following.

**Tools and libraries.**

- Verus and the Z3 solver are sound, and Verus's model of Rust matches the
  compiler. Verus reads the code with its own Rust 1.100.0 front
  end; production compiles the same source with Rust 1.90.
- `vstd`'s specifications of the standard library the kernels use are right,
  among them:
  - `Vec`, slice indexing, sub-slicing and equality;
  - `String` (`new`, `push`, `as_str`, `clone`), string literals, `Chars` and
    the character view of `&str`, and UTF-8 encoding;
  - `char::is_whitespace` and `char::to_ascii_lowercase`;
  - integer `wrapping_mul`, `checked_mul` and `checked_add`;
  - `Option::unwrap_or`;
  - `sort_by` (used by `groups`).

  One `vstd` function with an unverified body, `vstd::slice::slice_to_vec`,
  runs in production (used by `components`).
- `cargo verus verify` compiles the same crates.io `vstd` that the production
  build uses (`=0.0.0-2026-11-22-0344`), so proofs and production share one `vstd`.
  That version is the one the pinned Verus release expects.

**Oracles production implements.** Kernels reach effects through traits that
production implements, and Verus trusts each implementation's postconditions.
Traits whose postconditions only record that a call happened are satisfied by
any implementation. For them, the proofs describe real effects only if each
method does what it names.

| Trait (kernel module) | Production implementation | What production must get right |
| --- | --- | --- |
| `Identity` (`publish::trace`) | `Ident` in `src/fs.rs` (Unix only) | `same_inode` compares exactly `(st_dev, st_ino)`, the pair `Identity::ino` abstracts and the same comparison `name_holds_inode` makes when it proves a name for the kernel. |
| `RuleMatchOracle` (`rules::precedence`) | `ContextMatchers` in `src/worktreeinclude.rs` | `contexts` and `ancestors` count the applicable rule files and the path's non-empty ancestors (parent first), `query_path(ctx)` returns rule file `ctx`'s `Gitignore::matched_path_or_any_parents` verdict for the path, and `query_ancestor(anc, ctx)` returns the glob exactly when `Gitignore::matched(ancestor, true)` is an `Ignore` (selecting) match, all fixed for one evaluation. |
| `LevelMatchOracle` (`exclusion`) | `ExclusionLevels` in `src/policy_filter.rs` | `query_level(l)` returns the exclusion matcher's `Gitignore::matched` verdict for level `l` (the path with its own `is_dir`, then each parent as a directory, ending with the empty path), or a single level holding `matched_path_or_any_parents`'s verdict when `ignore`'s root strip could change a level, with `levels` and every answer fixed for one candidate. |
| `RuleFileProbe` (`selection`) | `RuleFilePath` in `src/worktreeinclude.rs` | `probe_is_file` reports whether the rule-file path resolves, following symlinks, to a regular file and `probe_is_symlink` whether the path itself is a symlink (false when its metadata cannot be read), both stable for one decision. |
| `DiagnosisFacts` (`selection`) | `DiagnosisProbes` in `src/subcommands/mod.rs` | Each probe reports the fact it names (the root `.worktreeinclude` is a symlink, false when its metadata cannot be read; `root_rule_file_is_consulted`, the root-only engine's gate, admits it; `GitBackend::worktreeinclude_exists_anywhere` under `follow` finds a rule file, or the error that walk failed with), stable for one diagnosis. |
| `FoldOracle` (`tracked`) | `UnicodeCaseFolding` in `src/git.rs` | `fold_key` is a deterministic function of the path bytes alone, so index entries and queries fold identically; which spellings count as case variants is whatever this fold says, while an exact index spelling is recognized without it. |
| `IdentityOracle` (`tracked`) | `FilesystemIdentityProbe` in `src/git.rs` | `query_identity` returns the filesystem identity of the repository path under the source root (`symlink_metadata` device and inode on Unix; on Windows the volume serial number and file index of a handle opened on the path, which follows a final symlink; none elsewhere), or `None` when it does not resolve, stable for one kernel call. |
| `PathKey` (`groups`) | `RepoRelPath` (the impl is in `src/eligibility_groups.rs`) | `key_bytes` returns the same bytes (the path's UTF-8 bytes) for the same value on every call, so sorting and deduplicating by key are well defined. |
| `EntryRunner` (`execute`) | `PlanRunner` in `src/executor.rs` | Records each call; any implementation satisfies the contract, and the proofs describe real effects only if `publish` attempts one publication of the entry and returns its outcome, and each `report_*` pushes one result row saying what its row says. |
| `AcquireOps` (`index_lock`) | `LockAttempts` in `src/executor.rs` | Records each call; any implementation satisfies the contract, and the proofs (and the `index_lock::model` theorems, tied to the drivers only by review) describe real effects only if each method performs the step it names, with `try_create` one `create_new` that treats `AlreadyExists`, and only it, as contention and records a created file's descriptor in `LOCK_FD`. Off Unix, `stage`, `arm` and `disarm` do nothing and `check_deferred` always returns `Ok`. |
| `StageOps` (`index_lock`) | `interrupt_cleanup::Stager` in `src/executor.rs` (Unix only) | Records each call; any implementation satisfies the contract, and the proofs (and the `index_lock::model` theorems, tied to the drivers only by review) describe real effects only if each method is the one operation it names: a `SeqCst` store to `STATE` or `LOCK_FD`, handler installation (once per process), or the copy of the path bytes and a terminating NUL into `PATH`, which `stage` allows only after checking the length production passes it. |
| `SignalCells` (`index_lock`) | `interrupt_cleanup::Cells` in `src/executor.rs` (Unix only) | Records each call (the `after_lock_removed` test seam records nothing and is empty outside tests); any implementation satisfies the contract, and the proofs (and the `index_lock::model` theorems, tied to the drivers only by review) describe real effects only if each other method is one async-signal-safe operation: a `SeqCst` access to `STATE` or `PENDING_SIGNAL`, the unlink that proceeds only when `fstat(LOCK_FD)` and `lstat(PATH)` report the same `(st_dev, st_ino)`, or restore-and-raise. |
| `IndexCacheIo` (`git_policy`) | `IndexStateEffects` in `src/git.rs` | Records each call; any implementation satisfies the contract, and the proofs describe real effects only if `observe_index` stats the index when called, `look_up` returns what the last `store` put in this repository's slot, `store` replaces that slot, and `build` reads the index as it stands. The proofs speak about stamps (identity, size, modification time); that an unchanged stamp means unchanged index contents is assumed. |
| `ConfigPathReader` (`context::cli_policy`) | `ProcessConfigPath` in `src/cli.rs` | Records each call; any implementation satisfies the contract, and the proofs describe real effects only if `read_config_path_var` classifies `std::env::var("WAFT_CONFIG_PATH")` faithfully, with `empty` equal to `value.is_empty()`. |
| `BoundaryProbes` (`boundary`) | `FsBoundaryProbes` in `src/walk.rs` | Records each call; any implementation satisfies the contract, and the proofs describe real effects only if each probe answers what its `BoundaryProbe` variant asks (`.git` is a directory; `.git` is a file whose `gitdir:` target exists; the directory's repository path matches a registered gitlink under `git::repo_paths_equivalent`, folding case when told to). |
| `EnvLookup` (`policy::user_source`) | `ProcessConfigHome` in `src/config.rs` | Records each call; any implementation satisfies the contract, and the proofs describe real effects only if `is_set` reports `std::env::var_os(name).is_some()` and keeps that value, which `user_config_path` reads after the kernel returns. |
| `DiscoveryWalk` (`policy::discovery`) | `ConfigSearchWalk` in `src/config.rs` | Records each call; any implementation satisfies the contract, and the proofs describe real effects only if `enter(i)` descends into component `i` and reports whether `<dir>/.git` exists (following symlinks) or the directory is a registered gitlink, and `search` records the current directory. |

**Conversions and call sites.** Production lowers its own types to kernel
types (path components, snapshots, enums, error kinds) with exhaustive matches,
most pinned by unit tests. Each conversion must be faithful.

**Step machines.** Typestates make production feed answers back in the order
the protocol requires, and keep it from building kernel states directly. They
do not show that an answer came from the named system call. Unverified code
could still produce any state, including `Ready` or a `Removed` outcome, from
made-up answers. Production is trusted to feed real answers.

**Models.** The following are proved about hand-written models:
- the signal-safety properties of the index lock;
- the safety theorems of the publication protocols;
- where the `git` rule engine agrees with Git and where it does not;
- the `-z` parser round trips;
- the index-cache provenance theorem;
- the `source_kind` label theorem.

A few postconditions are stated over hand-written models of `std`. The models
are reviewed and, where noted, pinned by tests against the real
implementation, but they are not derived from the operating system, `std`, Git
or the `ignore` crate.

**Platform.** No kernel code is platform-conditional. The gate, the script and
CI, runs on Linux x86-64 and verifies every kernel. Production calls some
kernels only on Unix, matching [ASSURANCE.md](ASSURANCE.md)'s Windows caveat:
publication, lock cleanup, the descriptor walk, errno classification, the
staging-name budget and the directory-creation steps. It calls the
verbatim-prefix kernel only on Windows.

**Execution.** Verus rejects kernel code that could reach an explicit panic
(`panic!`, `unwrap` of `None`, a failing `assert!`), overflow an integer, index
out of bounds, or fail to terminate. Every loop and recursive function carries
a `decreases` clause that Verus checks. Verus does not rule out unwinding from
library calls that are not marked `no_unwind`, and no kernel function is.
Growing a `Vec` or `String` (`push`, `slice_to_vec`) can still panic on
capacity overflow or abort when memory runs out. Postconditions describe
normal returns only, and production code around the kernels can still panic.

## Outside the verified boundary

Each area section lists the decision logic still in its production files. The
inventory for this record found three kinds:

- **I/O and effects (about 50 items).** System calls, file reads and writes,
  Git subprocesses and `gix` calls, environment reads, locks, signals and
  output. Verus does not model the operating system, so this code is trusted.
  The kernels shrink it to "perform this named effect and report the answer",
  and the test suite and [ASSURANCE.md](ASSURANCE.md) cover it.
- **Outside Verus's reach here (about 25 items).** This logic depends on:
  - other crates' semantics: `gix` (index, excludes, discovery), `ignore` (the
    gitignore matcher both rule engines query), `clap` parsing, `toml` and
    `serde` decoding, `reflink-copy`, `unicode-casefold`;
  - `std::path` parsing and `PathBuf` re-parsing, and `OsStr` encodings;
  - concurrency between processes beyond the modeled protocols;
  - panics and unwinding.

  The kernels take these as oracle answers or as already-lowered data.
- **Could move into a kernel with more work (about 45 items).** This is pure
  decision logic left in production, usually because it is small, is mostly
  formatting, or would need standard-library operations this Verus release has
  no specifications for. Examples:
  - assembling manifests and plans from kernel answers;
  - the planner's directory-tree recursion;
  - which files `validate` reads, and the global-excludes lookup;
  - the walkers' candidate filter;
  - `wt-0.39` negation subtraction;
  - parts of context probing;
  - error-to-default mappings for Git configuration reads;
  - message and table rendering.

Verus limitations that shaped the boundary, on this release:

- No specifications for many `str` methods (`starts_with`, `split`, `trim`,
  `strip_prefix`), `String::len`, string ordering, `Vec::dedup` and
  `Vec::retain`, ordered maps keyed by `String`, `&dyn Fn` and function
  pointers.
- No modeling of `std::path`, `OsStr` or I/O.

Kernels work on byte slices and lowered enums instead, and `--no-cheating`
rules out wrapping those APIs in trusted specifications.

## How the proofs were checked

A proof is only as good as its specification. Every area therefore went through
an adversarial review after it was written, a fix pass, and an independent audit
of the fixes. The merged tree was then checked as a whole at commit `2d47a55`.
Two later changes, one shared symlink-policy model and one shared
path-component model, were reviewed, mutation-tested and audited on their own.
On the final tree the Verus gate, the test suite and the end-to-end comparison
pass.

- **Claims audit.** Reviewers checked each claimed theorem against its Verus
  text: the quantifiers, the conditions, and whether it is a postcondition of
  the function production calls or a lemma about a model. Of 317 claims, 284
  were accurate, 32 overstated and 1 wrong. All 33 were corrected, either by
  proving the stronger statement or by correcting the documentation.
- **Mutation testing.** Reviewers changed kernel code on purpose and re-ran
  Verus: flipped conditions, shifted bounds, dropped or duplicated steps, wrong
  fields or payloads, early returns. A specification that still verifies a
  wrong program is too weak.
  - First pass: 823 mutants. Verus rejected 780; 14 were equivalent to the
    original; 27 exposed specification gaps; 2 did not compile. Every gap was
    closed, and each such mutant was re-run until Verus rejected it.
  - Resample on the merged tree at `2d47a55`: 306 mutant runs across 43 of the
    46 modules that contain code. Verus rejected 300; 3 were equivalent; 3 were
    pinned only by another module's proofs or by no independent theorem (see
    below).
- **Vacuity probes.** An `assert(false)` inserted into a proof or a function
  with preconditions must fail to verify. If it verifies, the preconditions
  contradict each other and the proof says nothing. The resample placed one or
  two probes per module, at the newest lemmas and at exec functions with
  preconditions: 77 in all, and all failed as required. The fixers and auditors
  probed the lemmas they added or changed.
- **Seams.** Integration checks covered the places where areas meet.
  - Changing how the executor classifies an up-to-date entry, a skip, a
    failure, a dry-run copy or a copy without a destination root breaks both
    the executor's proofs and the dry-run tally's proofs.
  - Changing which counter a publication result goes to breaks only the
    executor's proofs, because the tally theorems use only that it is one of
    the publication counters.
  - Changing the planner's outcome (copy, skip or failure) breaks the
    `list`/`info` prediction theorem. Changing only the kind of copy does not,
    by design.
  - Every clause of the tally-to-executor bridge theorem is needed.
- **Review findings.** 2 high and 10 medium findings were fixed, all
  independently audited. Among them:
  - the executor's ghost log did not record which entry each event was for;
  - the fallback replacement's final verdict was not tied to the names its
    cleanup actually consumed;
  - a cleanup guard could be re-armed for another file;
  - a configuration error variant was unconstrained;
  - the Git index cache step was composed in unverified code;
  - several cross-kernel claims rested on hand-written mirrors of another
    kernel.

Known limits of the specifications themselves:

- Several kernels state their contract as an exact transcription of the
  decision table they replace: the configuration vocabularies and presets,
  `spec_resolve_roots`, `selection`'s routing table, and the index-lock driver
  effects. A coordinated change to such a table and its code still verifies.
  Independent theorems in each module pin the properties that matter, for
  example that the reported error is the violated requirement of lowest rank
  (highest precedence) and that quiet output never hides a failure. Unit tests
  pin the tables.
- The index-lock safety theorems are stated in terms of the deferral rule
  itself. Deferring a signal in more states would still satisfy them; it would
  change behavior, not safety.
- Two choices are pinned only from outside their own module:
  - a planner skip reason is pinned only by `report_view`'s planner theorem,
    not by the planner's own theorems;
  - which publication counter the executor assigns is pinned only by the
    executor's own proofs; the dry-run tally's theorems do not constrain it.

## Behavior and cost

**Behavior.** The kernels were written to mirror the decision code they
replace, including quirks the area sections name. That they match it rests on
review and testing, not proof. The one intended change: a reachable
`unreachable!` panic in `RepoRelPath::normalize` now returns the existing
"outside the repository root" error. One trigger is normalizing `../x` against
the root `/`.

Evidence that nothing else changed:

- **Test suite.** The full suite passes on every merge.
- **Differential tests.** Reviewers checked each production diff against the
  code before the kernels. Committed property tests compare old and new code
  on 512 to 2,048 generated cases each. Reviewers also ran temporary
  comparisons with up to several hundred thousand cases, for example 505,440
  context-resolution combinations and 200,000 path-normalization inputs.
- **End-to-end comparison.** Release builds of `main` and of this branch ran
  side by side at `c25c8b9`, the last code change: 153
  scenarios, most of them once with each Git backend (267 scenario runs), for
  2,321 invocation pairs.
  - The scenarios cover every subcommand and the main flags, rule-file
    variants, file states, configuration files, environment variables, odd
    paths and repository layouts.
  - Every pair matched: output, exit status, the source and destination trees,
    `git status`, and leftover lock or staging names. Runs that panic or hang
    do so in both binaries.
  - The run used Linux as root on ext4 and tmpfs with one Git version. It did
    not exercise permission-denied paths, macOS or Windows, the
    exchange-unsupported fallback, real reflinks, concurrent writers or races,
    signal-time lock cleanup, or a repository at `/` (the intended change).

**Cost.** Build times vary by tens of seconds on the shared 4-CPU machine used;
the times below are from one quiet pair of clean builds.

| | `main` | this branch |
| --- | --- | --- |
| Release binary (Linux x86-64) | 6,608,864 bytes | about 6,720,000 bytes (+1.7%) |
| Clean release build | about 39 s | about 44 s (nine extra crates: `waft-verified`, `vstd`, `verus_builtin`, `verus_builtin_macros`, `verus_state_machines_macros`, `verus_syn`, `verus_prettyplease`, `synstructure`, `convert_case`) |
| Verus check (`scripts/verus-verify.sh`) | none | about 25-30 s, after a one-time verification of `vstd` |

## Defects found along the way

Writing specifications for existing behavior surfaced latent defects in waft.
They range from selection corner cases and odd-filesystem robustness to
misleading diagnostics, and a few could lose data or crash. Apart from the
panic noted under [Behavior and cost](#behavior-and-cost), this work did not
fix them, so behavior stayed unchanged. They were reported to the maintainer
privately, following [SECURITY.md](SECURITY.md).

## Maintaining the proofs

- **New decisions.** Put a new decision in `waft-verified`, inside `verus!`.
  Give the function production calls a postcondition that fixes its result, or
  the exact sequence of effects it requests, and call it from `src/`.
- **Preconditions.** Prefer no `requires` on functions production calls. When
  one is needed, make the fact it needs a postcondition of another kernel, so
  that production carries it forward.
- **Platforms.** Keep kernel code free of `cfg` conditions, so the Linux Verus
  run verifies what every platform executes.
- **Step machines.** Keep step-machine fields private and their types free of
  `Clone` and `Copy`.
- **Specification changes.** After changing a specification, check that it
  still rejects wrong code: mutate the code, re-run Verus, and make sure it
  fails.
- **Production glue.** When changing production code that calls
  `waft_verified`, implements one of its traits, or converts values to or from
  its types, keep the [Trust boundary](#trust-boundary) and the area section
  accurate.
- Run `just check-verus` before pushing; CI runs it too.
