//! Alt key tips: tap Alt and every ribbon tab shows a letter; press it and the commands of that
//! tab show a key each; press one to run it. Pure logic so the letters are tested.

/// Keys for `n` commands in the order they appear: 1-9, then A-Z (at most 35).
pub fn command_keys(n: usize) -> Vec<String> {
    "123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ"
        .chars()
        .take(n)
        .map(String::from)
        .collect()
}

/// A letter per label: the first letter not already taken, falling back to later letters.
pub fn letters(labels: &[&str]) -> Vec<char> {
    let mut used: Vec<char> = Vec::new();
    labels
        .iter()
        .map(|l| {
            let pick = l
                .chars()
                .filter(|c| c.is_ascii_alphabetic())
                .map(|c| c.to_ascii_uppercase())
                .find(|c| !used.contains(c))
                .or_else(|| ('A'..='Z').find(|c| !used.contains(c)))
                .unwrap_or('?');
            used.push(pick);
            pick
        })
        .collect()
}

/// Which level the tips are at.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Tips {
    #[default]
    Off,
    /// Letters on the tabs.
    Tabs,
    /// Keys on the commands of the open tab.
    Commands,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_unique_and_follow_the_label_where_they_can() {
        let l = letters(&["Features", "Sketch", "Evaluate", "View"]);
        assert_eq!(l, ['F', 'S', 'E', 'V']);
        // A clash moves on to the next letter of the label, then to any free one.
        assert_eq!(letters(&["Sketch", "Solid"]), ['S', 'O']);
        let l = letters(&["a", "a", "a"]);
        assert_eq!(l, ['A', 'B', 'C']);
        let mut u = l.clone();
        u.sort_unstable();
        u.dedup();
        assert_eq!(u.len(), 3);
    }

    #[test]
    fn command_keys_run_one_to_nine_then_letters_and_stop_at_the_end() {
        let k = command_keys(12);
        assert_eq!(k[0], "1");
        assert_eq!(k[8], "9");
        assert_eq!(k[9], "A");
        assert_eq!(command_keys(100).len(), 35);
        assert!(command_keys(0).is_empty());
    }
}
