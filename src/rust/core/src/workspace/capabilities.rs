//! Linux capabilities, as the kernel reports them in the `CapEff:` line of `/proc/<pid>/status` (a
//! hex bit mask). Which capabilities the container holds is the compose file's decision; this lets
//! a command that needs one it was not given say so, plainly, instead of failing deep inside
//! `debootstrap` or `chroot` with "Operation not permitted".

pub const CAP_NET_RAW: u8 = 13;
pub const CAP_SYS_CHROOT: u8 = 18;
pub const CAP_MKNOD: u8 = 27;

/// The effective capability set in the text of a `/proc/<pid>/status` file.
pub fn effective_set(status: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("CapEff:"))
        .and_then(|hex| u64::from_str_radix(hex.trim(), 16).ok())
}

/// Whether capability number `cap` is in the set.
pub fn has(set: u64, cap: u8) -> bool {
    cap < 64 && set & (1u64 << cap) != 0
}

/// The names, out of `wanted` (number, name), that the set lacks.
pub fn missing<'a>(set: u64, wanted: &[(u8, &'a str)]) -> Vec<&'a str> {
    wanted
        .iter()
        .filter(|(cap, _)| !has(set, *cap))
        .map(|(_, name)| *name)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = "Name:\tsdw\nCapInh:\t0000000000000000\nCapPrm:\t00000000a80425fb\n\
                          CapEff:\t00000000a80425fb\nCapBnd:\t00000000a80425fb\n";

    #[test]
    fn the_effective_set_is_read_from_the_status_text() {
        assert_eq!(effective_set(STATUS), Some(0xa804_25fb));
        assert_eq!(effective_set("Name:\tx\n"), None);
        assert_eq!(effective_set("CapEff:\tnot-hex\n"), None);
        assert_eq!(effective_set(""), None);
    }

    #[test]
    fn a_bit_is_a_capability() {
        // 0xa80425fb is Docker's default set: it has CHOWN (0), SETUID (7), NET_RAW (13),
        // SYS_CHROOT (18) and MKNOD (27), and lacks SYS_ADMIN (21) and NET_ADMIN (12).
        let set = effective_set(STATUS).unwrap();
        assert!(has(set, 0) && has(set, 7));
        assert!(has(set, CAP_NET_RAW) && has(set, CAP_SYS_CHROOT) && has(set, CAP_MKNOD));
        assert!(!has(set, 21) && !has(set, 12));
        // Out-of-range numbers are not held (and do not overflow the shift).
        assert!(!has(u64::MAX, 64));
    }

    #[test]
    fn missing_names_what_is_absent() {
        let wanted = [(CAP_SYS_CHROOT, "SYS_CHROOT"), (CAP_MKNOD, "MKNOD")];
        assert!(missing(0xa804_25fb, &wanted).is_empty());
        assert_eq!(missing(0, &wanted), ["SYS_CHROOT", "MKNOD"]);
        assert_eq!(missing(1 << CAP_MKNOD, &wanted), ["SYS_CHROOT"]);
    }
}
