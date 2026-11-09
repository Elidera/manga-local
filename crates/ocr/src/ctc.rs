//! Reading the recogniser's output: for each time step the likeliest symbol;
//! repeats merged, the blank (index 0) dropped (CTC greedy decoding).

/// `logits`: T × C probabilities (the PaddleOCR models end in softmax).
/// `chars[i]` is the symbol of class i (0 = blank). Returns the text and the
/// mean probability of its symbols.
pub fn decode(logits: &[f32], t: usize, c: usize, chars: &[String]) -> (String, f32) {
    let mut text = String::new();
    let (mut sum, mut n) = (0.0f32, 0usize);
    let mut prev = usize::MAX;
    for step in 0..t {
        let row = &logits[step * c..(step + 1) * c];
        let (best, p) = row.iter().enumerate().fold((0, f32::MIN), |acc, (i, &v)| if v > acc.1 { (i, v) } else { acc });
        if best != 0 && best != prev {
            if let Some(s) = chars.get(best) {
                text.push_str(s);
                sum += p;
                n += 1;
            }
        }
        prev = best;
    }
    (text, if n == 0 { 0.0 } else { sum / n as f32 })
}

/// The symbol list of a PaddleOCR recogniser from its embedded alphabet
/// (one symbol per line): blank first, the alphabet, a space last.
/// A trailing line break in the embedded list is not a symbol: counted as
/// one it would shift the space onto an empty slot and words would run
/// together ("You'relateagain").
pub fn alphabet(embedded: &str) -> Vec<String> {
    let mut v = vec![String::new()];
    v.extend(embedded.split('\n').map(|s| s.trim_end_matches('\r').to_string()));
    while v.len() > 1 && v.last().is_some_and(|s| s.is_empty()) {
        v.pop();
    }
    v.push(" ".into());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeats_merge_and_blanks_split() {
        let chars = alphabet("a\nb");
        // Classes: 0 blank, 1 a, 2 b, 3 space. Steps: a a blank a b space b.
        let seq = [1, 1, 0, 1, 2, 3, 2];
        let c = 4;
        let mut logits = vec![0.0f32; seq.len() * c];
        for (t, &k) in seq.iter().enumerate() {
            logits[t * c + k] = 0.9;
        }
        let (s, p) = decode(&logits, seq.len(), c, &chars);
        assert_eq!(s, "aab b");
        assert!((p - 0.9).abs() < 1e-5);
    }

    #[test]
    fn trailing_line_break_keeps_the_space_last() {
        assert_eq!(alphabet("a\nb\n"), alphabet("a\nb"));
        assert_eq!(alphabet("a\r\nb\r\n").len(), 4);
    }
}
