/// Fuzzes SFNT/TTC directory validation, table bounds, cmap, names, and
/// collection face selection.
pub fn fuzz_sfnt_directory(data: &[u8]) {
    aimer_cupid::fuzz_aimer_font_directory(data);
}

/// Fuzzes TrueType composites and CFF/CFF2 Type 2 outlines.
pub fn fuzz_font_outlines(data: &[u8]) {
    aimer_cupid::fuzz_aimer_font_outlines(data);
}

#[cfg(test)]
mod tests {
    use super::{fuzz_font_outlines, fuzz_sfnt_directory};

    #[test]
    fn harnesses_accept_empty_and_high_entropy_inputs() {
        fuzz_sfnt_directory(&[]);
        fuzz_sfnt_directory(&[0xff; 4 * 1024]);
        fuzz_font_outlines(&[]);
        fuzz_font_outlines(&[0xff; 4 * 1024]);
    }
}
