//! User config location: which environment variable provides the base
//! directory of the default user config file.
//!
//! # What it decides
//!
//! [`select_config_home`] (called by production `user_config_path`)
//! chooses among `XDG_CONFIG_HOME`, `HOME` and `USERPROFILE`, querying them
//! lazily through the [`EnvLookup`] oracle.
//!
//! # Theorems (postcondition of `select_config_home`)
//!
//! The variables are queried in priority order, querying stops at the first
//! variable that is set (later ones are never read), and the result is that
//! variable; `None` means all three were queried and none was set.
//!
//! # Trust
//!
//! The oracle records each query in its ghost log; its answers are
//! arbitrary. Production reads `std::env::var_os`, keeps the value of the
//! variable it last queried, and appends `waft/config.toml` (XDG) or
//! `.config/waft/config.toml` (HOME, USERPROFILE). The precedence of
//! `--isolated`, `--config` and `WAFT_CONFIG_PATH` over this default is
//! decided by the verified `context::cli_policy::user_config_source`
//! kernel, not by this one; production consults this kernel only when that
//! one selects the default location.
//!
//! Quirk (current behavior): being set is the only test, so an empty or
//! relative value is used as is and the user layer can then resolve
//! relative to the process working directory.

#[allow(unused_imports)]
use vstd::prelude::*;

verus! {

/// An environment variable that can provide the user config base directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigHomeVar {
    /// `XDG_CONFIG_HOME`: the file is `$XDG_CONFIG_HOME/waft/config.toml`.
    XdgConfigHome,
    /// `HOME`: the file is `$HOME/.config/waft/config.toml`.
    Home,
    /// `USERPROFILE`: the file is `%USERPROFILE%/.config/waft/config.toml`.
    UserProfile,
}

impl ConfigHomeVar {
    /// The variable's name.
    pub open spec fn name(self) -> Seq<char> {
        match self {
            ConfigHomeVar::XdgConfigHome => "XDG_CONFIG_HOME"@,
            ConfigHomeVar::Home => "HOME"@,
            ConfigHomeVar::UserProfile => "USERPROFILE"@,
        }
    }

    /// The variable's name.
    pub fn as_str(self) -> (s: &'static str)
        ensures
            s@ == self.name(),
    {
        match self {
            ConfigHomeVar::XdgConfigHome => "XDG_CONFIG_HOME",
            ConfigHomeVar::Home => "HOME",
            ConfigHomeVar::UserProfile => "USERPROFILE",
        }
    }
}

/// The variables in the order they are consulted.
pub open spec fn priority() -> Seq<ConfigHomeVar> {
    seq![ConfigHomeVar::XdgConfigHome, ConfigHomeVar::Home, ConfigHomeVar::UserProfile]
}

/// Reads environment variables for [`select_config_home`] (implemented by
/// production over the process environment).
pub trait EnvLookup {
    /// Ghost model: the variables queried so far and whether each was set.
    spec fn queries(&self) -> Seq<(ConfigHomeVar, bool)>;

    /// Whether `var` is set (production: `std::env::var_os(name).is_some()`,
    /// remembering the value). Trusted: the answer is recorded in the model.
    fn is_set(&mut self, var: ConfigHomeVar) -> (set: bool)
        ensures
            final(self).queries() == old(self).queries().push((var, set)),
    ;
}

/// Chooses the variable that provides the user config base directory: the
/// first set variable in `priority` order, or none.
///
/// Proven: the variables are queried in priority order, querying stops at
/// the first one that is set (later ones are never read), and the result is
/// that variable; `None` means all three were queried and none was set.
/// Being set is the only test: like `std::env::var_os(..).is_some()`, an
/// empty or relative value counts as set (see the module documentation of
/// `policy` for this quirk).
pub fn select_config_home<L: EnvLookup>(lookup: &mut L) -> (r: Option<ConfigHomeVar>)
    ensures
        ({
            let q = final(lookup).queries().skip(old(lookup).queries().len() as int);
            &&& final(lookup).queries().take(old(lookup).queries().len() as int) == old(
                lookup,
            ).queries()
            &&& 1 <= q.len() <= 3
            &&& forall|j: int| 0 <= j < q.len() ==> (#[trigger] q[j]).0 == priority()[j]
            &&& forall|j: int| 0 <= j < q.len() - 1 ==> !(#[trigger] q[j]).1
            &&& (r matches Some(v) ==> q.last() == (v, true))
            &&& (r is None ==> q.len() == 3 && !q.last().1)
        }),
{
    let ghost q0 = lookup.queries();
    if lookup.is_set(ConfigHomeVar::XdgConfigHome) {
        proof {
            assert(lookup.queries().skip(q0.len() as int) =~= seq![(ConfigHomeVar::XdgConfigHome, true)]);
            assert(lookup.queries().take(q0.len() as int) =~= q0);
        }
        return Some(ConfigHomeVar::XdgConfigHome);
    }
    let ghost q1 = lookup.queries();
    if lookup.is_set(ConfigHomeVar::Home) {
        proof {
            assert(lookup.queries().skip(q0.len() as int) =~= seq![
                (ConfigHomeVar::XdgConfigHome, false),
                (ConfigHomeVar::Home, true),
            ]);
            assert(lookup.queries().take(q0.len() as int) =~= q0);
        }
        return Some(ConfigHomeVar::Home);
    }
    let set = lookup.is_set(ConfigHomeVar::UserProfile);
    proof {
        assert(lookup.queries().skip(q0.len() as int) =~= seq![
            (ConfigHomeVar::XdgConfigHome, false),
            (ConfigHomeVar::Home, false),
            (ConfigHomeVar::UserProfile, set),
        ]);
        assert(lookup.queries().take(q0.len() as int) =~= q0);
    }
    if set {
        Some(ConfigHomeVar::UserProfile)
    } else {
        None
    }
}

} // verus!
