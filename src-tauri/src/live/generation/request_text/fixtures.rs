use crate::live::{
    LiveCourseInfo, LiveSummaryChunk, LiveTermExplanation, LiveTranscriptLine, SharedSummaryChunk,
    SharedTranscriptLine,
};

pub struct Input {
    pub course: LiveCourseInfo,
    pub lines: Vec<SharedTranscriptLine>,
    pub summaries: Vec<SharedSummaryChunk>,
}

pub fn input(line_count: usize, summary_count: usize, seed: usize) -> Input {
    const TEXT: [&str; 6] = [
        "",
        "ASCII | : \"quote\" \\",
        "日本語 中文\n引用",
        " 👩🏽‍💻 e\u{301} ",
        "\t\r\n　",
        "# title\n- item\n",
    ];
    Input {
        course: LiveCourseInfo {
            course_name: TEXT[seed % 6].into(),
            course_code: TEXT[(seed + 1) % 6].into(),
            teacher: TEXT[(seed + 2) % 6].into(),
            room: TEXT[(seed + 3) % 6].into(),
            day: [-1, 0, 1, 7, i32::MIN, i32::MAX][seed % 6],
            period: (seed % 6) as i32,
            time_label: TEXT[(seed + 4) % 6].into(),
            is_free_note: seed % 2 == 1,
        },
        lines: (0..line_count)
            .map(|index| {
                LiveTranscriptLine {
                    text: format!("line{index}:{}", TEXT[(seed + index) % 6].repeat(index % 4)),
                    at: TEXT[(seed + index + 1) % 6].into(),
                }
                .into()
            })
            .collect(),
        summaries: (0..summary_count)
            .map(|index| {
                LiveSummaryChunk {
                    title: format!("summary{index}:{}", TEXT[(seed + index) % 6]),
                    range_label: TEXT[(seed + index + 1) % 6].into(),
                    body: format!(
                        "body{index}:{}",
                        TEXT[(seed + index + 2) % 6].repeat(index % 4)
                    ),
                    line_count: index,
                    terms: vec![LiveTermExplanation {
                        term: "unused term".into(),
                        explanation: "unused explanation".into(),
                        source_excerpt: String::new(),
                        external_source: String::new(),
                    }],
                    whiteboard: None,
                }
                .into()
            })
            .collect(),
    }
}
