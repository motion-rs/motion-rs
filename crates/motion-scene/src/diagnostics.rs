//! Friendlier diagnostics: source locations for JSON paths and "did you mean" hints.

use motion_core::ValidationError;

/// Largest edit distance for which a candidate is offered as a suggestion.
const MAX_DISTANCE: usize = 3;

/// A step of a JSON path: an object key or an array index.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Step {
    Key(String),
    Index(usize),
}

/// Returns the 1-based line and column of the deepest value on `path` that exists in `text`.
///
/// A path such as `tracks[0].items[1].scene.content.fill` follows object keys and array
/// indices. When a step does not exist, for example because a `$param` object stands in
/// for the value, the location of the last step that does is returned.
pub(crate) fn locate(text: &str, path: &str) -> Option<(usize, usize)> {
    let steps = steps(path);
    let bytes = text.as_bytes();
    let mut scanner = Scanner { bytes, position: 0 };
    scanner.skip_space();
    let mut value = scanner.position;
    let mut shown = value;
    for step in steps {
        match scanner.child(value, &step) {
            Some((start, child)) => {
                shown = start;
                value = child;
            }
            None => break,
        }
    }
    let before = &text[..shown.min(text.len())];
    let line = before.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count())
        + 1;
    Some((line, column))
}

/// Appends the source location and any suggestion to a diagnostic's message.
///
/// Unknown-key and unknown-variant messages locate the offending key itself when it
/// is present in the object at `path`.
pub(crate) fn annotate(mut error: ValidationError, text: &str) -> ValidationError {
    let mut path = error.path.split(' ').next().unwrap_or_default().to_owned();
    let unknown = quoted_after(&error.message, "unknown field `").map(str::to_owned);
    if let Some(name) = &unknown
        && let Some(parent) = path.strip_suffix(&format!(".{name}"))
    {
        // Serde reports the unknown key as part of the path; the object is the location.
        path = parent.to_owned();
        error.path = path.clone();
        error.field = name.clone();
    }
    let target = match &unknown {
        Some(name) if path == "document" => name.clone(),
        Some(name) => format!("{path}.{name}"),
        None => path,
    };
    let hint = serde_suggestion(&error.message);
    if let Some(shortened) = shorten_expected(&error.message) {
        error.message = shortened;
    }
    if let Some((line, column)) = locate(text, &target) {
        error
            .message
            .push_str(&format!(" (line {line}, column {column})"));
    }
    if let Some(hint) = hint {
        error.message.push_str(&format!(" {hint}"));
    }
    error
}

/// Largest list of valid names that is still worth printing in a diagnostic.
const MAX_LISTED_NAMES: usize = 8;

/// Drops serde's "expected one of ..." list when it is too long to help.
fn shorten_expected(message: &str) -> Option<String> {
    let (head, list) = message.split_once(", expected one of ")?;
    let count = list.split('`').skip(1).step_by(2).count();
    (count > MAX_LISTED_NAMES).then(|| format!("{head}; {count} names are valid here."))
}

/// Maps every diagnostic inside `report` through [`annotate`].
pub(crate) fn annotate_report(
    report: motion_core::ErrorReport,
    text: &str,
) -> motion_core::ErrorReport {
    report.map(|error| match error.downcast_ref::<ValidationError>() {
        Some(diagnostic) => std::sync::Arc::new(annotate(diagnostic.clone(), text).into()),
        None => error,
    })
}

/// Returns "Did you mean `x`?" for the closest candidate, if any is close enough.
pub(crate) fn did_you_mean<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let limit = MAX_DISTANCE.min(name.chars().count().saturating_sub(1));
    candidates
        .into_iter()
        .map(|candidate| (distance(name, candidate), candidate))
        .filter(|(distance, candidate)| *distance <= limit && *candidate != name)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| format!("Did you mean `{candidate}`?"))
}

/// Suggests a fix for serde's unknown field and variant messages.
fn serde_suggestion(message: &str) -> Option<String> {
    let name = quoted_after(message, "unknown field `")
        .or_else(|| quoted_after(message, "unknown variant `"))?;
    let expected = message
        .split_once("expected one of ")
        .or_else(|| message.split_once("expected "))?
        .1;
    let candidates: Vec<&str> = expected.split('`').skip(1).step_by(2).collect();
    did_you_mean(name, candidates.iter().copied())
}

fn quoted_after<'m>(message: &'m str, prefix: &str) -> Option<&'m str> {
    let rest = message.split_once(prefix)?.1;
    rest.split_once('`').map(|(name, _)| name)
}

/// Levenshtein distance over characters.
fn distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut row: Vec<usize> = (0..=right.len()).collect();
    for (index, left) in left.chars().enumerate() {
        let mut previous = row[0];
        row[0] = index + 1;
        for (column, right) in right.iter().enumerate() {
            let substitution = previous + usize::from(left != *right);
            previous = row[column + 1];
            row[column + 1] = substitution.min(row[column] + 1).min(row[column + 1] + 1);
        }
    }
    row[right.len()]
}

fn steps(path: &str) -> Vec<Step> {
    let path = path.split(' ').next().unwrap_or_default();
    let mut steps = Vec::new();
    if path == "document" {
        return steps;
    }
    for part in path.split('.') {
        let (key, mut rest) = match part.find('[') {
            Some(open) => (&part[..open], &part[open..]),
            None => (part, ""),
        };
        if !key.is_empty() {
            steps.push(Step::Key(key.to_owned()));
        }
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some((index, tail)) = stripped.split_once(']') else {
                break;
            };
            match index.parse() {
                Ok(index) => steps.push(Step::Index(index)),
                Err(_) => break,
            }
            rest = tail;
        }
    }
    steps
}

/// A minimal JSON walker that finds where child values start without building a tree.
struct Scanner<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Scanner<'_> {
    /// Returns where `step` is shown (its key, or the element) and where its value starts.
    fn child(&mut self, at: usize, step: &Step) -> Option<(usize, usize)> {
        self.position = at;
        match (self.bytes.get(at)?, step) {
            (b'{', Step::Key(wanted)) => {
                self.position += 1;
                loop {
                    self.skip_space();
                    if self.bytes.get(self.position) != Some(&b'"') {
                        return None;
                    }
                    let key_start = self.position;
                    let key = self.string()?;
                    self.skip_space();
                    if self.bytes.get(self.position) != Some(&b':') {
                        return None;
                    }
                    self.position += 1;
                    self.skip_space();
                    if key == *wanted {
                        return Some((key_start, self.position));
                    }
                    self.value()?;
                    self.skip_space();
                    match self.bytes.get(self.position)? {
                        b',' => self.position += 1,
                        _ => return None,
                    }
                }
            }
            (b'[', Step::Index(wanted)) => {
                self.position += 1;
                for index in 0.. {
                    self.skip_space();
                    if index == *wanted {
                        return Some((self.position, self.position));
                    }
                    self.value()?;
                    self.skip_space();
                    match self.bytes.get(self.position)? {
                        b',' => self.position += 1,
                        _ => return None,
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Skips one value, returning `None` for malformed input.
    fn value(&mut self) -> Option<()> {
        self.skip_space();
        match self.bytes.get(self.position)? {
            b'"' => {
                self.string()?;
            }
            b'{' | b'[' => {
                let mut depth = 0usize;
                while let Some(byte) = self.bytes.get(self.position) {
                    match byte {
                        b'"' => {
                            self.string()?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                self.position += 1;
                                return Some(());
                            }
                        }
                        _ => {}
                    }
                    self.position += 1;
                }
                return None;
            }
            _ => {
                while self
                    .bytes
                    .get(self.position)
                    .is_some_and(|byte| !b",}] \t\r\n".contains(byte))
                {
                    self.position += 1;
                }
            }
        }
        Some(())
    }

    /// Reads a string at the current quote, returning its unescaped text.
    fn string(&mut self) -> Option<String> {
        let start = self.position;
        self.position += 1;
        while let Some(byte) = self.bytes.get(self.position) {
            match byte {
                b'\\' => self.position += 2,
                b'"' => {
                    self.position += 1;
                    let raw = std::str::from_utf8(&self.bytes[start..self.position]).ok()?;
                    return serde_json::from_str(raw).ok();
                }
                _ => self.position += 1,
            }
        }
        None
    }

    fn skip_space(&mut self) {
        while self
            .bytes
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str =
        "{\n  \"a\": [1, {\"b\": \"x]\"}],\n  \"c\": {\n    \"d\": true, \"e_k\": 5\n  }\n}";

    #[test]
    fn paths_locate_keys_indices_and_fall_back_to_the_deepest_prefix() {
        assert_eq!(locate(TEXT, "a[1].b"), Some((2, 13)));
        assert_eq!(locate(TEXT, "c.d"), Some((4, 5)));
        assert_eq!(locate(TEXT, "c.e_k"), Some((4, 16)));
        assert_eq!(locate(TEXT, "c.missing.deeper"), Some((3, 3)));
        assert_eq!(locate(TEXT, "a[9]"), Some((2, 3)));
        assert_eq!(locate(TEXT, "c"), Some((3, 3)));
        assert_eq!(locate("[]", "x"), Some((1, 1)));
    }

    #[test]
    fn suggestions_use_close_candidates_only() {
        assert_eq!(
            did_you_mean("colour", ["color", "size"]),
            Some("Did you mean `color`?".into())
        );
        assert_eq!(did_you_mean("zzz", ["color", "size"]), None);
        assert_eq!(did_you_mean("x", ["y"]), None);
        assert_eq!(did_you_mean("size", ["size"]), None);
        assert_eq!(
            serde_suggestion("unknown field `colour`, expected one of `color`, `size`"),
            Some("Did you mean `color`?".into())
        );
        assert_eq!(
            serde_suggestion("unknown variant `cubc_out`, expected one of `linear`, `cubic_out`"),
            Some("Did you mean `cubic_out`?".into())
        );
    }

    #[test]
    fn annotation_points_at_the_offending_key() {
        let text = "{\n  \"colour\": 1\n}";
        let error = ValidationError::invalid(
            "document",
            "json",
            "unknown field `colour`, expected one of `color`, `size`",
        );
        let message = annotate(error, text).message;
        assert!(message.contains("(line 2, column 3)"), "{message}");
        assert!(message.ends_with("Did you mean `color`?"), "{message}");
    }
}
