//! Verified kernel: policy vocabulary, compat-profile presets, configuration
//! layer merging, and the decisions around building layers.
//!
//! Production (`src/config.rs`) keeps its own enums (they derive
//! `clap::ValueEnum`) and lowers them to these kernels with exhaustive
//! variant-for-variant conversions; serde/toml parsing, environment access,
//! `std::path` handling, filesystem probes, and error-message formatting stay
//! in production. Every decision listed below is made by kernel code that
//! production calls; the replaced production logic was deleted.
//!
//! # Submodules and theorems
//!
//! - [`vocab`]: the accepted and printed spellings of each policy enum.
//!   `as_str` returns the canonical name, `parse` equals the declarative
//!   `spec_parse` ("the unique variant spelled `s`"), and
//!   `theorem_*_vocabulary` proves `all()` lists every variant once, names
//!   are injective, parsing is exact (case-sensitive, untrimmed), the round
//!   trip `parse(as_str(x)) == x`, and that each `EXPECTED` error hint is the
//!   canonical names joined with `|` in declaration order.
//!   `theorem_copy_strategy_vocabulary`/`_aliases` add that copy-strategy
//!   spellings (canonical plus `default`, `simple`, `cow`, `reflink`) of
//!   different variants are disjoint.
//! - [`merge`]: `resolve_layers` and `resolve_inputs` are proven equal to the
//!   closed form `spec_resolve` / `spec_extras`, and
//!   `theorem_closed_form_is_fold` proves the closed form equals the
//!   documented operational rule. Further theorems:
//!   `theorem_profile_last_writer` (P1), `theorem_knob_last_writer` and
//!   `theorem_profile_shadows_lower_layers` (P2: an explicit knob below the
//!   last profile selection never matters), `theorem_copy_strategy_independent`
//!   (P3), `theorem_extras_closed_form` and
//!   `theorem_extras_depend_only_on_replace_flags` (P4: extras are the layers'
//!   extras from the last replacing layer on, in order; `Some(false)` acts as
//!   `None`), `theorem_neutral_layer` and
//!   `theorem_neutral_layers_resolve_to_default` plus
//!   `theorem_default_resolution` (P5), `theorem_preset_table` (P6: only `wt`
//!   selects without a rule file; presets are distinct),
//!   `theorem_precedence` (P7: CLI over environment over project over user
//!   over defaults, a CLI profile resets lower knobs, CLI replace keeps only
//!   CLI extras), and `theorem_attribution` (P8: a knob that differs from the
//!   resolved profile's preset was set explicitly, to that value, at or after
//!   the last profile selection; this is what lets the empty-selection note
//!   blame the knob instead of the profile).
//! - [`raw`]: `layer_from_raw` decides config-file acceptance after serde:
//!   the version gate first, then the first unknown value in a fixed field
//!   order, otherwise each field's parse.
//! - [`env_layer`]: `EnvKey::classify` recognizes exactly the eight `WAFT_*`
//!   policy variables (`theorem_env_key_table`), and `parse_bool_token`
//!   decides `WAFT_REPLACE_EXTRA_EXCLUDES` on the trimmed, ASCII-lowercased
//!   value (`theorem_bool_tokens_disjoint`).
//! - [`user_source`]: `select_config_home` queries `XDG_CONFIG_HOME`, `HOME`,
//!   `USERPROFILE` in that order, stops at the first one that is set, and
//!   returns it.
//! - [`discovery`]: `walk_toward_invocation` decides which directories from
//!   the source root toward the invocation directory are searched for
//!   `.waft.toml`; with `theorem_reachable_names`, only name components
//!   before the first `..`/root/prefix component are entered, in order, at
//!   most once, and nothing at or below a nested checkout or registered
//!   gitlink is searched.
//!
//! # Trust assumptions
//!
//! - The production conversions (`From` impls, `ConfigLayer::kernel_view`,
//!   `ConfigLayer::from_kernel`, `ResolvedPolicy::from_kernel`,
//!   `component_kind`) map variants and fields one to one. They are
//!   exhaustive matches and destructurings, so a new variant or field fails
//!   to compile; unit tests check the round trips and that clap's value names
//!   equal `as_str`.
//! - vstd's specifications of `&str` equality, string literals, slice
//!   indexing, `Vec` and `String::clone`.
//! - The oracles: `user_source::EnvLookup::is_set` and
//!   `discovery::DiscoveryWalk::{enter, search}` are assumed to record each
//!   call in their ghost log (a modelling device: no production state is
//!   assumed). What an oracle answers is not trusted to be anything in
//!   particular: the theorems hold for every answer. Production's oracles
//!   read the process environment, and probe `.git` existence and registered
//!   gitlinks (Unicode case folding in `git::repo_paths_equivalent`).
//! - Production keeps `str::trim` and `str::to_ascii_lowercase` before
//!   `parse_bool_token`, the `WAFT_EXTRA_EXCLUDE` comma split, serde/toml
//!   deserialization, and all error-message text.
//!
//! # Quirks encoded faithfully (current behavior, not fixed here)
//!
//! - Being *set* is the only test for the user config base variables: an
//!   empty or relative `XDG_CONFIG_HOME`, `HOME` or `USERPROFILE` is used as
//!   is, so the user layer can resolve relative to the process working
//!   directory.
//! - A non-name component in the invocation path ends the discovery walk but
//!   keeps the directories already searched (`a/../b` searches the root and
//!   `a`), although callers pass normalized paths.
//! - Copy-strategy aliases are accepted from TOML and the environment but
//!   are not advertised in the error hint (and the CLI flag rejects them).

pub mod discovery;
pub mod env_layer;
pub mod merge;
pub mod raw;
pub mod user_source;
pub mod vocab;
