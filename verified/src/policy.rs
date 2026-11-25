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
//!   `theorem_precedence` (P7: an explicit CLI setting wins over every lower
//!   layer, a CLI profile resets the knobs the CLI leaves unset to its
//!   preset, an explicit environment knob wins when the CLI sets neither that
//!   knob nor a profile, and CLI replace keeps only CLI extras. It says
//!   nothing about project, user and defaults against each other: their
//!   order comes from `resolve_inputs` resolving the layer sequence
//!   `precedence_order` (defaults, user, project, environment, CLI), in
//!   which P1 and P2 make the last explicit setting win as long as no later
//!   layer selects a profile), and
//!   `theorem_attribution` (P8: a knob that differs from the
//!   resolved profile's preset was set explicitly, to that value, at or after
//!   the last profile selection; this is what lets the empty-selection note
//!   blame the knob instead of the profile).
//! - [`raw`]: `layer_from_raw` decides config-file acceptance after serde:
//!   the version gate first (and `UnsupportedVersion` only for a bad
//!   version), then `UnknownValue` naming the first bad field in a fixed
//!   field order with its raw value, otherwise each field's parse;
//!   `theorem_first_bad_field` shows these postconditions fix the result
//!   for every input.
//! - [`env_layer`]: `EnvKey::classify` recognizes exactly the eight `WAFT_*`
//!   policy variables (`theorem_env_key_table`), `parse_env_bool` decides
//!   `WAFT_REPLACE_EXTRA_EXCLUDES` on the value trimmed of Unicode whitespace
//!   and ASCII-lowercased (`theorem_bool_tokens_disjoint`: the true and false
//!   spellings are disjoint), and `parse_exclude_list` is
//!   proven equal to splitting `WAFT_EXTRA_EXCLUDE` on commas, trimming
//!   Unicode whitespace and dropping empty pieces
//!   (`theorem_exclude_list_globs`: every glob is non-empty, trimmed, and
//!   comma-free).
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
//! - vstd's specifications of `&str` equality, string literals, `Chars`,
//!   slice indexing, `Vec`, `String::push`, `String::clone`, and its
//!   `White_Space` table for `char::is_whitespace`.
//! - The oracles: `user_source::EnvLookup::is_set` and
//!   `discovery::DiscoveryWalk::{enter, search}` are assumed to record each
//!   call in their ghost log (a modelling device: no production state is
//!   assumed). What an oracle answers is not trusted to be anything in
//!   particular: the theorems hold for every answer. Production's oracles
//!   read the process environment, and probe `.git` existence and registered
//!   gitlinks (Unicode case folding in `git::repo_paths_equivalent`).
//! - Production keeps serde/toml deserialization, the iteration over the
//!   process environment, and all error-message text (including the
//!   `str::trim`/`str::to_ascii_lowercase` echo in the invalid-boolean
//!   message).
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
