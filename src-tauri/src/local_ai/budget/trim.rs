use crate::ai::ChatMessage;

use super::json::{compact_embedded_json, compact_json_value, outermost_brace_spans};
use super::tokens::estimate_apple_tokens;

fn split_atoms(text: &str) -> Vec<String> {
    let spans = outermost_brace_spans(text);
    let mut atoms = Vec::new();
    let mut cursor = 0usize;
    for (start, end) in spans {
        if start > cursor {
            push_prose_atoms(&mut atoms, &text[cursor..start]);
        }
        atoms.push(text[start..end].to_string());
        cursor = end;
    }
    if cursor < text.len() {
        push_prose_atoms(&mut atoms, &text[cursor..]);
    }
    atoms.retain(|atom| !atom.trim().is_empty());
    atoms
}

fn push_prose_atoms(out: &mut Vec<String>, prose: &str) {
    if prose.is_empty() {
        return;
    }
    let parts: Vec<&str> = prose.split("\n\n").collect();
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if index == 0 {
            out.push((*part).to_string());
        } else {
            out.push(format!("\n\n{part}"));
        }
    }
}

fn is_brace_atom(atom: &str) -> bool {
    let trimmed = atom.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn fit_atom(atom: &str, budget: usize, keep_tail: bool) -> Option<String> {
    if budget < 2 || atom.is_empty() {
        return None;
    }
    if estimate_apple_tokens(atom) <= budget {
        return Some(atom.to_string());
    }
    if is_brace_atom(atom) {
        let trimmed = atom.trim();
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
            let compact = compact_json_value(&value, budget);
            let rendered = serde_json::to_string(&compact).unwrap_or_else(|_| "{}".into());
            if estimate_apple_tokens(&rendered) <= budget {
                return Some(rendered);
            }
        }
        return None;
    }
    if atom.contains('{') || atom.contains('[') {
        return None;
    }
    let sliced = slice_prose(atom, budget, keep_tail);
    if sliced.trim().is_empty() {
        None
    } else {
        Some(sliced)
    }
}

fn slice_prose(text: &str, budget: usize, keep_tail: bool) -> String {
    if estimate_apple_tokens(text) <= budget {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut low = 0usize;
    let mut high = chars.len();
    while low < high {
        let mid = (low + high + 1) / 2;
        let slice: String = if keep_tail {
            chars[chars.len() - mid..].iter().collect()
        } else {
            chars[..mid].iter().collect()
        };
        if estimate_apple_tokens(&slice) <= budget {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    if keep_tail {
        chars[chars.len() - low..].iter().collect()
    } else {
        chars[..low].iter().collect()
    }
}

fn join_atoms(atoms: &[String]) -> String {
    let mut out = String::new();
    let mut omitted = false;
    for atom in atoms {
        if atom.is_empty() {
            omitted = true;
            continue;
        }
        if omitted && !out.is_empty() && !out.ends_with('…') {
            out.push_str("\n…\n");
        }
        omitted = false;
        out.push_str(atom);
    }
    out
}

fn enforce_atom_budget(mut atoms: Vec<String>, budget: usize, drop_front: bool) -> String {
    while estimate_apple_tokens(&join_atoms(&atoms)) > budget && !atoms.is_empty() {
        if drop_front {
            atoms.remove(0);
        } else {
            atoms.pop();
        }
    }
    join_atoms(&atoms)
}

fn atom_room(budget: usize, used: usize) -> usize {
    let separator = usize::from(used > 0);
    budget.saturating_sub(used).saturating_sub(separator)
}

fn select_side(atoms: &[String], budget: usize, keep_tail: bool) -> String {
    let mut chosen = Vec::new();
    let mut used = 0usize;
    let indexes: Vec<usize> = if keep_tail {
        (0..atoms.len()).rev().collect()
    } else {
        (0..atoms.len()).collect()
    };
    for index in indexes {
        let separator = usize::from(used > 0);
        let remaining = atom_room(budget, used);
        let Some(fitted) = fit_atom(&atoms[index], remaining, keep_tail) else {
            continue;
        };
        let cost = estimate_apple_tokens(&fitted) + separator;
        if used + cost > budget {
            continue;
        }
        used += cost;
        if keep_tail {
            chosen.insert(0, fitted);
        } else {
            chosen.push(fitted);
        }
    }
    enforce_atom_budget(chosen, budget, keep_tail)
}

fn select_head_and_tail(atoms: &[String], budget: usize) -> String {
    let mut chosen: Vec<Option<String>> = vec![None; atoms.len()];
    let mut used = 0usize;
    for (index, atom) in atoms.iter().enumerate() {
        if !is_brace_atom(atom) {
            continue;
        }
        let separator = usize::from(used > 0);
        let remaining = atom_room(budget, used);
        let Some(fitted) = fit_atom(atom, remaining, false) else {
            continue;
        };
        let cost = estimate_apple_tokens(&fitted) + separator;
        if used + cost > budget {
            continue;
        }
        used += cost;
        chosen[index] = Some(fitted);
    }
    let prose_budget = budget.saturating_sub(used);
    let head_limit = prose_budget / 3;
    let mut prose_used = 0usize;
    for (index, atom) in atoms.iter().enumerate() {
        if chosen[index].is_some() || is_brace_atom(atom) {
            continue;
        }
        let separator = usize::from(prose_used > 0);
        let remaining = atom_room(head_limit, prose_used);
        let Some(fitted) = fit_atom(atom, remaining, false) else {
            break;
        };
        let cost = estimate_apple_tokens(&fitted) + separator;
        if prose_used + cost > head_limit {
            break;
        }
        prose_used += cost;
        chosen[index] = Some(fitted);
    }
    for (index, atom) in atoms.iter().enumerate().rev() {
        if chosen[index].is_some() || is_brace_atom(atom) {
            continue;
        }
        let separator = usize::from(prose_used > 0);
        let remaining = atom_room(prose_budget, prose_used);
        let Some(fitted) = fit_atom(atom, remaining, true) else {
            continue;
        };
        let cost = estimate_apple_tokens(&fitted) + separator;
        if prose_used + cost > prose_budget {
            continue;
        }
        prose_used += cost;
        chosen[index] = Some(fitted);
    }
    let kept: Vec<String> = chosen.into_iter().flatten().collect();
    enforce_atom_budget(kept, budget, false)
}

/// Keep rules, JSON blocks, and the latest material. JSON is never cut mid-value.
pub(crate) fn trim_apple_text(text: &str, budget: usize) -> String {
    trim_structured(text, budget, false)
}

pub(in crate::local_ai) fn trim_apple_turns(prompt: &str, budget: usize) -> String {
    trim_structured(prompt, budget, true)
}

fn trim_structured(text: &str, budget: usize, keep_tail: bool) -> String {
    if budget == 0 || text.is_empty() {
        return String::new();
    }
    let compacted = compact_embedded_json(text, budget.max(32));
    if estimate_apple_tokens(&compacted) <= budget {
        return compacted;
    }
    let atoms = split_atoms(&compacted);
    let selected = if keep_tail {
        select_side(&atoms, budget, true)
    } else {
        select_head_and_tail(&atoms, budget)
    };
    if selected.is_empty() && !compacted.contains('{') && !compacted.contains('[') {
        return slice_prose(&compacted, budget, keep_tail);
    }
    selected
}

pub(crate) fn apple_request_parts(messages: &[ChatMessage], prefill: &str) -> (String, String) {
    let mut instructions = Vec::new();
    let mut turns = Vec::new();
    for message in messages {
        let content = message.content.trim();
        if content.is_empty() {
            continue;
        }
        match message.role.as_str() {
            "system" | "developer" => instructions.push(content.to_string()),
            "assistant" => turns.push(format!("Assistant:\n{content}")),
            _ => turns.push(format!("User:\n{content}")),
        }
    }
    let prefill = prefill.trim();
    if !prefill.is_empty() {
        instructions.push(
            "応答は下書きの続きだけを出力し、下書き自体は繰り返さないでください。".to_string(),
        );
        turns.push(format!("Assistant:\n{prefill}"));
    }
    (instructions.join("\n\n"), turns.join("\n\n"))
}
