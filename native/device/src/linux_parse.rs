//! Parsers for the text files a Linux machine describes itself with.
//!
//! They take text rather than paths so they are compiled — and tested — on
//! every host, not just on the machines that have these files.

use crate::DeviceKind;

/// The fields of `os-release(5)` the device description uses.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct OsRelease {
    pub(crate) name: Option<String>,
    pub(crate) id: Option<String>,
    pub(crate) pretty_name: Option<String>,
    pub(crate) version_id: Option<String>,
    pub(crate) build_id: Option<String>,
}

/// Parses the shell-style `KEY=value` lines of an `os-release` file.
///
/// Values may be unquoted or wrapped in single or double quotes; comments and
/// blank lines are skipped, and unknown keys ignored.
pub(crate) fn parse_os_release(text: &str) -> OsRelease {
    let mut release = OsRelease::default();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(value.trim());
        if key.starts_with('#') || value.is_empty() {
            continue;
        }
        let slot = match key.trim() {
            "NAME" => &mut release.name,
            "ID" => &mut release.id,
            "PRETTY_NAME" => &mut release.pretty_name,
            "VERSION_ID" => &mut release.version_id,
            "BUILD_ID" => &mut release.build_id,
            _ => continue,
        };
        *slot = Some(value.to_owned());
    }
    release
}

fn unquote(value: &str) -> &str {
    ['"', '\'']
        .into_iter()
        .find_map(|quote| value.strip_prefix(quote)?.strip_suffix(quote))
        .unwrap_or(value)
}

/// Maps an SMBIOS chassis type (`/sys/class/dmi/id/chassis_type`) to a form
/// factor. Anything that is not a handheld or a tablet is a [`Desktop`]
/// (laptops, towers, all-in-ones, servers), since a Linux desktop session is
/// what runs this code.
///
/// [`Desktop`]: DeviceKind::Desktop
pub(crate) fn kind_from_chassis_type(code: u32) -> DeviceKind {
    match code {
        // Hand Held.
        11 => DeviceKind::Phone,
        // Tablet and Detachable.
        30 | 32 => DeviceKind::Tablet,
        _ => DeviceKind::Desktop,
    }
}

/// Whether `/proc/cpuinfo` lists the `hypervisor` CPU flag, which every
/// mainstream hypervisor sets for its guests.
pub(crate) fn has_hypervisor_flag(cpuinfo: &str) -> bool {
    cpuinfo
        .lines()
        .filter_map(|line| line.strip_prefix("flags"))
        .filter_map(|flags| flags.split_once(':'))
        .any(|(_, flags)| flags.split_whitespace().any(|flag| flag == "hypervisor"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const UBUNTU: &str = "\
PRETTY_NAME=\"Ubuntu 24.04.1 LTS\"
NAME=\"Ubuntu\"
VERSION_ID=\"24.04\"
VERSION=\"24.04.1 LTS (Noble Numbat)\"
ID=ubuntu
ID_LIKE=debian
";

    #[test]
    fn reads_quoted_and_bare_values() {
        let release = parse_os_release(UBUNTU);
        assert_eq!(release.name.as_deref(), Some("Ubuntu"));
        assert_eq!(release.id.as_deref(), Some("ubuntu"));
        assert_eq!(release.pretty_name.as_deref(), Some("Ubuntu 24.04.1 LTS"));
        assert_eq!(release.version_id.as_deref(), Some("24.04"));
        assert_eq!(release.build_id, None);
    }

    #[test]
    fn skips_comments_blanks_and_handles_single_quotes() {
        let release = parse_os_release("# a comment\n\nNAME='Arch Linux'\nBUILD_ID=rolling\n");
        assert_eq!(release.name.as_deref(), Some("Arch Linux"));
        assert_eq!(release.build_id.as_deref(), Some("rolling"));
        assert_eq!(release.id, None);
    }

    #[test]
    fn empty_and_malformed_input_yields_nothing() {
        assert_eq!(parse_os_release(""), OsRelease::default());
        assert_eq!(parse_os_release("not a pair\nNAME=\n"), OsRelease::default());
    }

    #[test]
    fn chassis_types_map_to_form_factors() {
        assert_eq!(kind_from_chassis_type(3), DeviceKind::Desktop);
        assert_eq!(kind_from_chassis_type(10), DeviceKind::Desktop);
        assert_eq!(kind_from_chassis_type(11), DeviceKind::Phone);
        assert_eq!(kind_from_chassis_type(30), DeviceKind::Tablet);
        assert_eq!(kind_from_chassis_type(32), DeviceKind::Tablet);
        assert_eq!(kind_from_chassis_type(9999), DeviceKind::Desktop);
    }

    #[test]
    fn detects_the_hypervisor_flag_as_a_whole_word() {
        let guest = "processor\t: 0\nflags\t\t: fpu vme sse2 hypervisor lahf_lm\n";
        let host = "processor\t: 0\nflags\t\t: fpu vme sse2 vmx lahf_lm\n";
        let lookalike = "flags\t\t: fpu nothypervisor\n";
        assert!(has_hypervisor_flag(guest));
        assert!(!has_hypervisor_flag(host));
        assert!(!has_hypervisor_flag(lookalike));
        assert!(!has_hypervisor_flag(""));
    }
}
