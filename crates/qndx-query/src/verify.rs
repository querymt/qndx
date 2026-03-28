//! Verification pass: match regex against candidate file contents.

use regex::Regex;

/// Verify which files from the candidate set actually match the pattern.
pub fn verify_candidates(pattern: &str, candidates: &[(usize, &[u8])]) -> Vec<usize> {
    let re = match Regex::new(pattern) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    candidates
        .iter()
        .filter_map(|(id, content)| {
            // Search content as UTF-8 lossy
            let text = String::from_utf8_lossy(content);
            if re.is_match(&text) {
                Some(*id)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_basic() {
        let files = vec![
            (0usize, b"hello world MAX_FILE_SIZE = 100" as &[u8]),
            (1, b"nothing here"),
            (2, b"MAX_FILE_SIZE is defined"),
        ];
        let matches = verify_candidates("MAX_FILE_SIZE", &files);
        assert_eq!(matches, vec![0, 2]);
    }

    #[test]
    fn verify_regex() {
        let files = vec![
            (0usize, b"foo123bar" as &[u8]),
            (1, b"fooxyzbar"),
            (2, b"bazqux"),
        ];
        let matches = verify_candidates(r"foo\d+bar", &files);
        assert_eq!(matches, vec![0]);
    }
}
