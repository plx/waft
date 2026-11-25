//! Verified kernel: command-line policy inputs.
//!
//! # What it decides
//!
//! - Which command a context is resolved for when no subcommand is given
//!   ([`command_kind`]).
//! - Which user configuration file is loaded, if any, and whether
//!   `WAFT_CONFIG_PATH` is read at all ([`user_config_source`]): none under
//!   `--isolated`; otherwise `--config`; otherwise a set, Unicode, non-empty
//!   `WAFT_CONFIG_PATH`; otherwise the default location. The kernel reads the
//!   variable itself, through the [`ConfigPathReader`] oracle, and only when
//!   neither flag decides.
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
//! - [`user_config_source`]'s postcondition: the result is `precedence` of
//!   the flags and of the value read, and `WAFT_CONFIG_PATH` is read exactly
//!   once when neither `--isolated` nor `--config` is given
//!   (`env_consulted`), and never otherwise. Production calls this function
//!   and loads whatever its result names, so the laziness is proven of the
//!   code that runs, not of a spec-level composition. Outside the kernel,
//!   production has no other `WAFT_CONFIG_PATH` read; the `WAFT_*` policy
//!   layer enumerates the whole environment but ignores this name (glue,
//!   not verified here).
//! - `theorem_user_config_precedence`: `precedence` in closed form.
//!   `--isolated` wins over everything (also over `--config`, so the kernel
//!   does not rely on clap rejecting that combination); otherwise `--config`
//!   wins; otherwise a set, Unicode, non-empty variable; otherwise the
//!   default location.
//! - `theorem_flags_make_env_irrelevant`: when a flag decides, no value of
//!   the variable could change the result, so not reading it loses nothing.
//! - [`env_config_choice`]'s postcondition: only a set, Unicode, non-empty
//!   `WAFT_CONFIG_PATH` is used; unset, empty *and non-Unicode* values all
//!   select the default location. The non-Unicode case is kept from the
//!   former code. It is not a silent fallback end to end: such a run is not
//!   isolated, so it goes on to read the `WAFT_*` policy layer with
//!   `std::env::vars()`, which panics on any non-Unicode variable (a
//!   pre-existing bug outside this kernel), unless an earlier error stops it
//!   first, for example a parse error in the default configuration file.
//! - [`policy_env_layer_applies`]: the `WAFT_*` policy layer applies
//!   exactly when not isolated.
//! - [`cli_replace_extra_excludes`] never yields `Some(false)`: the CLI layer
//!   can request a replace but never cancels one from a lower layer.
//!
//! # Trust assumptions
//!
//! - Production passes the flags as clap parsed them.
//! - The [`ConfigPathReader`] implementation classifies
//!   `std::env::var("WAFT_CONFIG_PATH")` faithfully (`Ok` gives `Set`, with
//!   `empty` equal to `value.is_empty()`; `NotPresent` gives `Unset`;
//!   `NotUnicode` gives `NotUnicode`). The kernel assumes only that each call
//!   is one read and is logged.

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
/// location (including, as kept from the former code, when the value is not
/// Unicode; see the module documentation for what such a run does next).
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

/// Reads `WAFT_CONFIG_PATH` for [`user_config_source`] (implemented by
/// production over the process environment).
pub trait ConfigPathReader<S> {
    /// Ghost model: every value read so far, oldest first.
    spec fn reads(&self) -> Seq<ConfigPathVar<S>>;

    /// Read and classify `WAFT_CONFIG_PATH` once. Trusted: production
    /// classifies `std::env::var` faithfully; the kernel assumes only that
    /// the read is logged.
    fn read_config_path_var(&mut self) -> (var: ConfigPathVar<S>)
        ensures
            final(self).reads() == old(self).reads().push(var),
    ;
}

/// Where the user configuration comes from.
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

/// Whether `WAFT_CONFIG_PATH` can matter: neither `--isolated` nor
/// `--config` decides.
pub open spec fn env_consulted<P>(isolated: bool, config_flag: Option<P>) -> bool {
    !isolated && config_flag is None
}

/// The precedence `--isolated` > `--config` > a set, Unicode, non-empty
/// `WAFT_CONFIG_PATH` > the default location. `var` is the value of the
/// variable as read, or `None` when it was not read.
pub open spec fn precedence<P, S>(
    isolated: bool,
    config_flag: Option<P>,
    var: Option<ConfigPathVar<S>>,
) -> UserConfigSource<P, S> {
    if isolated {
        UserConfigSource::Skipped
    } else if config_flag is Some {
        UserConfigSource::Flag(config_flag->Some_0)
    } else {
        match var {
            Some(ConfigPathVar::Set { value, empty: false }) => UserConfigSource::Env(value),
            _ => UserConfigSource::DefaultLocation,
        }
    }
}

/// Choose the user configuration source, reading `WAFT_CONFIG_PATH`
/// through `reader` only when neither flag decides.
pub fn user_config_source<P, S, R: ConfigPathReader<S>>(
    isolated: bool,
    config_flag: Option<P>,
    reader: &mut R,
) -> (r: UserConfigSource<P, S>)
    ensures
        ({
            let n = old(reader).reads().len() as int;
            let issued = final(reader).reads().skip(n);
            &&& final(reader).reads().take(n) == old(reader).reads()
            &&& issued.len() == (if env_consulted(isolated, config_flag) {
                1int
            } else {
                0int
            })
            &&& r == precedence(
                isolated,
                config_flag,
                if issued.len() == 1 {
                    Some(issued[0])
                } else {
                    None
                },
            )
        }),
{
    let ghost start = reader.reads();
    let ghost n = start.len() as int;
    if isolated {
        assert(reader.reads().skip(n) =~= Seq::<ConfigPathVar<S>>::empty());
        assert(reader.reads().take(n) =~= start);
        return UserConfigSource::Skipped;
    }
    match config_flag {
        Some(p) => {
            assert(reader.reads().skip(n) =~= Seq::<ConfigPathVar<S>>::empty());
            assert(reader.reads().take(n) =~= start);
            UserConfigSource::Flag(p)
        },
        None => {
            let var = reader.read_config_path_var();
            assert(reader.reads().skip(n) =~= seq![var]);
            assert(reader.reads().take(n) =~= start);
            match env_config_choice(var) {
                EnvConfigChoice::Path(s) => UserConfigSource::Env(s),
                EnvConfigChoice::DefaultLocation => UserConfigSource::DefaultLocation,
            }
        },
    }
}

/// `precedence` in closed form: each source is chosen exactly when every
/// higher-precedence source is absent and it is present.
pub proof fn theorem_user_config_precedence<P, S>(
    isolated: bool,
    config_flag: Option<P>,
    var: Option<ConfigPathVar<S>>,
)
    ensures
        precedence(isolated, config_flag, var) is Skipped <==> isolated,
        forall|p: P|
            #![auto]
            precedence(isolated, config_flag, var) == UserConfigSource::<P, S>::Flag(p) <==> (
            !isolated && config_flag == Some(p)),
        forall|s: S|
            #![auto]
            precedence(isolated, config_flag, var) == UserConfigSource::<P, S>::Env(s) <==> (
            env_consulted(isolated, config_flag) && var == Some(
                ConfigPathVar::Set { value: s, empty: false },
            )),
        precedence(isolated, config_flag, var) is DefaultLocation <==> (env_consulted(
            isolated,
            config_flag,
        ) && !(var matches Some(ConfigPathVar::Set { empty: false, .. }))),
{
}

/// When a flag decides, the value of `WAFT_CONFIG_PATH` cannot change the
/// result, so [`user_config_source`] loses nothing by not reading it.
pub proof fn theorem_flags_make_env_irrelevant<P, S>(
    isolated: bool,
    config_flag: Option<P>,
    a: Option<ConfigPathVar<S>>,
    b: Option<ConfigPathVar<S>>,
)
    requires
        !env_consulted(isolated, config_flag),
    ensures
        precedence(isolated, config_flag, a) == precedence(isolated, config_flag, b),
{
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

} // verus!
