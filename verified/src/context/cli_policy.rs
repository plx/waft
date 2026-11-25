//! Verified kernel: command-line policy inputs.
//!
//! # What it decides
//!
//! - Which command a context is resolved for when no subcommand is given
//!   ([`command_kind`]).
//! - Which user configuration file is loaded, if any: none under
//!   `--isolated`; otherwise `--config`; otherwise a non-empty
//!   `WAFT_CONFIG_PATH`; otherwise the default location
//!   ([`user_config_step`], then [`env_config_choice`]). The two-step shape
//!   keeps the environment variable unread unless it can matter.
//! - Whether the `WAFT_*` policy environment layer applies
//!   ([`policy_env_layer_applies`]).
//! - The CLI layer's `replace_extra_excludes` field
//!   ([`cli_replace_extra_excludes`]).
//!
//! # Theorems
//!
//! - [`command_kind`]'s postcondition: no subcommand resolves the context
//!   for `copy` (production's `dispatch` also runs `copy` then); each named
//!   subcommand resolves it for itself.
//! - [`user_config_step`]'s postcondition: under `--isolated` the step is
//!   `Skip` (no user configuration, `WAFT_CONFIG_PATH` not consulted);
//!   otherwise `--config` is used as given; otherwise the variable is
//!   consulted. [`policy_env_layer_applies`]: the `WAFT_*` policy layer
//!   applies exactly when not isolated.
//! - [`env_config_choice`]'s postcondition: only a set, Unicode, non-empty
//!   `WAFT_CONFIG_PATH` is used; unset, empty *and non-Unicode* values all
//!   fall back to the default location. The non-Unicode fallback is a
//!   documented quirk of the current behaviour (a value the user set is
//!   silently ignored), kept as is.
//! - `theorem_user_config_precedence`: composing the two steps gives the
//!   precedence `--isolated` > `--config` > `WAFT_CONFIG_PATH` > default,
//!   and the variable is never read when a flag decides.
//! - [`cli_replace_extra_excludes`] never yields `Some(false)`: the CLI layer
//!   can request a replace but never cancels one from a lower layer.
//!
//! # Trust assumptions
//!
//! - clap enforces `--config` conflicts with `--isolated`; production passes
//!   the parsed flags and classifies `std::env::var` faithfully.

#[allow(unused_imports)]
use vstd::prelude::*;

use super::CommandKind;

verus! {

/// The subcommand given on the command line, without its arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, StructuralEq)]
pub enum Subcommand {
    /// No subcommand (`waft` alone).
    None,
    /// `copy`.
    Copy,
    /// `list`.
    List,
    /// `info`.
    Info,
    /// `validate`.
    Validate,
}

/// The command a context is resolved for: `copy` when none is given.
pub fn command_kind(subcommand: Subcommand) -> (r: CommandKind)
    ensures
        r == match subcommand {
            Subcommand::None | Subcommand::Copy => CommandKind::Copy,
            Subcommand::List => CommandKind::List,
            Subcommand::Info => CommandKind::Info,
            Subcommand::Validate => CommandKind::Validate,
        },
{
    match subcommand {
        Subcommand::None | Subcommand::Copy => CommandKind::Copy,
        Subcommand::List => CommandKind::List,
        Subcommand::Info => CommandKind::Info,
        Subcommand::Validate => CommandKind::Validate,
    }
}

/// The first step of choosing the user configuration file.
pub enum UserConfigStep<P> {
    /// `--isolated`: no user configuration.
    Skip,
    /// Use the `--config` path.
    Flag(P),
    /// Consult `WAFT_CONFIG_PATH` (see [`env_config_choice`]).
    ConsultEnv,
}

/// Choose the user configuration source from the flags alone.
pub fn user_config_step<P>(isolated: bool, config_flag: Option<P>) -> (r: UserConfigStep<P>)
    ensures
        isolated ==> r is Skip,
        !isolated ==> match config_flag {
            Some(p) => r == UserConfigStep::Flag(p),
            None => r is ConsultEnv,
        },
{
    if isolated {
        return UserConfigStep::Skip;
    }
    match config_flag {
        Some(p) => UserConfigStep::Flag(p),
        None => UserConfigStep::ConsultEnv,
    }
}

/// The state of `WAFT_CONFIG_PATH`.
pub enum ConfigPathVar<S> {
    /// Not set.
    Unset,
    /// Set, but not valid Unicode.
    NotUnicode,
    /// Set to a Unicode value; `empty` tells whether it is the empty string.
    Set {
        /// The value.
        value: S,
        /// Whether the value is empty.
        empty: bool,
    },
}

/// Which user configuration path `WAFT_CONFIG_PATH` selects.
pub enum EnvConfigChoice<S> {
    /// Use this path from the environment.
    Path(S),
    /// Use the default user configuration location.
    DefaultLocation,
}

/// Use a set, Unicode, non-empty `WAFT_CONFIG_PATH`; otherwise the default
/// location (including, as a kept quirk, when the value is not Unicode).
pub fn env_config_choice<S>(var: ConfigPathVar<S>) -> (r: EnvConfigChoice<S>)
    ensures
        match var {
            ConfigPathVar::Set { value, empty } => if empty {
                r is DefaultLocation
            } else {
                r == EnvConfigChoice::Path(value)
            },
            _ => r is DefaultLocation,
        },
{
    match var {
        ConfigPathVar::Set { value, empty } => {
            if empty {
                EnvConfigChoice::DefaultLocation
            } else {
                EnvConfigChoice::Path(value)
            }
        },
        ConfigPathVar::Unset => EnvConfigChoice::DefaultLocation,
        ConfigPathVar::NotUnicode => EnvConfigChoice::DefaultLocation,
    }
}

/// Whether the `WAFT_*` policy environment layer applies.
pub fn policy_env_layer_applies(isolated: bool) -> (r: bool)
    ensures
        r == !isolated,
{
    !isolated
}

/// The CLI layer's `replace_extra_excludes`: `Some(true)` when the flag is
/// given, otherwise no opinion.
pub fn cli_replace_extra_excludes(flag: bool) -> (r: Option<bool>)
    ensures
        r == (if flag {
            Some(true)
        } else {
            None::<bool>
        }),
        r != Some(false),
{
    if flag {
        Some(true)
    } else {
        None
    }
}

/// Where the user configuration comes from, end to end.
pub enum UserConfigSource<P, S> {
    /// None (`--isolated`).
    Skipped,
    /// The `--config` path.
    Flag(P),
    /// The `WAFT_CONFIG_PATH` value.
    Env(S),
    /// The default user configuration location.
    DefaultLocation,
}

/// The two steps composed: what production loads, given the flags and what
/// reading `WAFT_CONFIG_PATH` *would* return (`None` models "not read").
pub open spec fn user_config_source<P, S>(
    step: UserConfigStep<P>,
    choice: Option<EnvConfigChoice<S>>,
) -> UserConfigSource<P, S> {
    match step {
        UserConfigStep::Skip => UserConfigSource::Skipped,
        UserConfigStep::Flag(p) => UserConfigSource::Flag(p),
        UserConfigStep::ConsultEnv => match choice {
            Some(EnvConfigChoice::Path(s)) => UserConfigSource::Env(s),
            _ => UserConfigSource::DefaultLocation,
        },
    }
}

/// The full precedence: `--isolated` skips the user configuration and the
/// policy environment layer and never reads `WAFT_CONFIG_PATH`; otherwise
/// `--config` wins without reading the variable; otherwise a set, Unicode,
/// non-empty variable wins; otherwise the default location is used.
pub proof fn theorem_user_config_precedence<P, S>(
    isolated: bool,
    config_flag: Option<P>,
    step: UserConfigStep<P>,
    var: ConfigPathVar<S>,
    choice: EnvConfigChoice<S>,
)
    requires
        isolated ==> step is Skip,
        !isolated ==> match config_flag {
            Some(p) => step == UserConfigStep::Flag(p),
            None => step is ConsultEnv,
        },
        match var {
            ConfigPathVar::Set { value, empty } => if empty {
                choice is DefaultLocation
            } else {
                choice == EnvConfigChoice::Path(value)
            },
            _ => choice is DefaultLocation,
        },
    ensures
        isolated ==> user_config_source::<P, S>(step, None) is Skipped && !(step is ConsultEnv),
        !isolated && config_flag is Some ==> user_config_source::<P, S>(step, None)
            == UserConfigSource::<P, S>::Flag(config_flag->Some_0) && !(step is ConsultEnv),
        !isolated && config_flag is None ==> step is ConsultEnv && user_config_source(
            step,
            Some(choice),
        ) == match var {
            ConfigPathVar::Set { value, empty: false } => UserConfigSource::<P, S>::Env(value),
            _ => UserConfigSource::<P, S>::DefaultLocation,
        },
{
}

} // verus!
