use super::super::*;

/// One raw source unit fed to the AI. Transient — these never appear in the
/// final `DetectiveCase`. The AI reads these and emits its own short
/// evidence cards (one paragraph of distilled information per card).
#[derive(Debug, Clone)]
pub(crate) struct EvidenceInputEntry {
    pub(crate) alias: String, // l1, s1, d1
    pub(crate) source_type: String,
    pub(crate) source: String,
    pub(crate) source_id: String,
    pub(crate) raw_title: String,
    pub(crate) raw_content: String,
    pub(crate) source_path: String,
    pub(crate) source_url: String,
    pub(crate) information_type: String,
    pub(crate) person_category_cd: String,
    pub(crate) category_cd: String,
}

/// Assemble the input package handed to the AI: full Live note bodies +
/// signal titles + doubt notes, each tagged with a short alias (l1/s1/d1).
pub(crate) fn build_evidence_input(course: &DetectiveCourse) -> Vec<EvidenceInputEntry> {
    let mut out = Vec::new();

    let mut li = 0u32;
    for record in course.live_records.iter().take(6) {
        li += 1;
        let content = if record.excerpt.trim().is_empty() {
            "(本文未抽出)".to_string()
        } else {
            record.excerpt.clone()
        };
        out.push(EvidenceInputEntry {
            alias: format!("l{li}"),
            source_type: "live".to_string(),
            source: "live".to_string(),
            source_id: record.id.clone(),
            raw_title: String::new(), // filename — deliberately not shown to AI
            raw_content: content,
            source_path: record.path.clone(),
            source_url: String::new(),
            information_type: String::new(),
            person_category_cd: String::new(),
            category_cd: String::new(),
        });
    }

    let mut si = 0u32;
    for signal in course.exam_signals.iter().take(6) {
        si += 1;
        let content = if signal.category.is_empty() {
            signal.title.clone()
        } else {
            format!("{}: {}", signal.category, signal.title)
        };
        out.push(EvidenceInputEntry {
            alias: format!("s{si}"),
            source_type: "signal".to_string(),
            source: signal.source.clone(),
            source_id: signal.source_id.clone(),
            raw_title: signal.title.clone(),
            raw_content: content,
            source_path: String::new(),
            source_url: signal.source_url.clone(),
            information_type: signal.information_type.clone(),
            person_category_cd: signal.person_category_cd.clone(),
            category_cd: signal.category_cd.clone(),
        });
    }

    let mut di = 0u32;
    for doubt in course.doubts.iter().take(3) {
        di += 1;
        out.push(EvidenceInputEntry {
            alias: format!("d{di}"),
            source_type: "doubt".to_string(),
            source: "doubt".to_string(),
            source_id: doubt.id.clone(),
            raw_title: String::new(),
            raw_content: doubt.note.clone(),
            source_path: String::new(),
            source_url: String::new(),
            information_type: String::new(),
            person_category_cd: String::new(),
            category_cd: String::new(),
        });
    }

    out
}

/// Assemble the AI input for ONE chapter: a single Live note as the primary
/// source (l1), plus the course's exam signals (s*) and doubts (d*) for
/// exam-context. Returns `None` if the live note can't be found.
pub(crate) fn build_chapter_input(
    course: &DetectiveCourse,
    live_id: &str,
) -> Option<Vec<EvidenceInputEntry>> {
    let record = course.live_records.iter().find(|r| r.id == live_id)?;
    let mut out = Vec::new();
    let content = if record.excerpt.trim().is_empty() {
        "(本文未抽出)".to_string()
    } else {
        record.excerpt.clone()
    };
    out.push(EvidenceInputEntry {
        alias: "l1".to_string(),
        source_type: "live".to_string(),
        source: "live".to_string(),
        source_id: record.id.clone(),
        raw_title: String::new(),
        raw_content: content,
        source_path: record.path.clone(),
        source_url: String::new(),
        information_type: String::new(),
        person_category_cd: String::new(),
        category_cd: String::new(),
    });

    let mut si = 0u32;
    for signal in course.exam_signals.iter().take(4) {
        si += 1;
        let content = if signal.category.is_empty() {
            signal.title.clone()
        } else {
            format!("{}: {}", signal.category, signal.title)
        };
        out.push(EvidenceInputEntry {
            alias: format!("s{si}"),
            source_type: "signal".to_string(),
            source: signal.source.clone(),
            source_id: signal.source_id.clone(),
            raw_title: signal.title.clone(),
            raw_content: content,
            source_path: String::new(),
            source_url: signal.source_url.clone(),
            information_type: signal.information_type.clone(),
            person_category_cd: signal.person_category_cd.clone(),
            category_cd: signal.category_cd.clone(),
        });
    }

    let mut di = 0u32;
    for doubt in course.doubts.iter().take(2) {
        di += 1;
        out.push(EvidenceInputEntry {
            alias: format!("d{di}"),
            source_type: "doubt".to_string(),
            source: "doubt".to_string(),
            source_id: doubt.id.clone(),
            raw_title: String::new(),
            raw_content: doubt.note.clone(),
            source_path: String::new(),
            source_url: String::new(),
            information_type: String::new(),
            person_category_cd: String::new(),
            category_cd: String::new(),
        });
    }

    Some(out)
}
