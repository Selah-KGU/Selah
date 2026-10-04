use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CampaignBibleDraft {
    pub(crate) world_label: Option<String>,
    pub(crate) setting: Option<String>,
    pub(crate) tagline: Option<String>,
    pub(crate) meta_mystery: Option<String>,
    pub(crate) cast: Option<Vec<CampaignCharacterDraft>>,
    pub(crate) meta_arc: Option<Vec<CampaignArcDraft>>,
    pub(crate) relationships: Option<Vec<CampaignRelationshipDraft>>,
    pub(crate) finale: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CampaignArcDraft {
    pub(crate) title: Option<String>,
    pub(crate) reveal: Option<String>,
    pub(crate) setup: Option<String>,
    pub(crate) misdirection: Option<String>,
    pub(crate) session_band: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CampaignRelationshipDraft {
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
    pub(crate) relation: Option<String>,
    pub(crate) tension: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CampaignCharacterDraft {
    pub(crate) name: Option<String>,
    pub(crate) role: Option<String>,
    pub(crate) bond: Option<String>,
    pub(crate) background: Option<String>,
    pub(crate) motivation: Option<String>,
    pub(crate) stake: Option<String>,
    pub(crate) voice: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetectiveAiCaseDraft {
    pub(crate) title: Option<String>,
    pub(crate) case_type: Option<String>,
    pub(crate) difficulty: Option<u8>,
    pub(crate) briefing: Option<String>,
    pub(crate) scenario: Option<String>,
    pub(crate) witness_name: Option<String>,
    pub(crate) witness_role: Option<String>,
    pub(crate) final_question: Option<String>,
    /// Which 授業計画 第N回 this Live note matches (content alignment); 0/absent
    /// when the model can't tell.
    pub(crate) session_num: Option<i32>,
    pub(crate) acts: Option<Vec<DetectiveAiActDraft>>,
    /// Where each knowledge point landed. Hard-validated against the must-cover
    /// list + COVERAGE_MIN_POINTS in `apply_ai_case_draft`.
    pub(crate) coverage: Option<Vec<CoverageDraft>>,
    /// The 推理 spine the draft realized (carried from the outline pass, may be
    /// refined by the draft / editor pass).
    pub(crate) case_logic: Option<CaseLogicDraft>,
    /// What this chapter contributes to the campaign's 暗线.
    pub(crate) meta_beat: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaseLogicDraft {
    pub(crate) truth: Option<String>,
    pub(crate) culprit: Option<String>,
    pub(crate) motive: Option<String>,
    pub(crate) red_herrings: Option<Vec<String>>,
    pub(crate) deduction_chain: Option<Vec<String>>,
}

/// Pass A output (the parts Rust reads back). The FULL outline — act plan,
/// coverage plan, per-act lie targets — is carried to the draft pass verbatim
/// as an embedded JSON string, so those fields don't need to round-trip through
/// Rust; only the logic spine / 暗线 beat / session number are reused here.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaseOutlineDraft {
    pub(crate) session_num: Option<i32>,
    pub(crate) case_logic: Option<CaseLogicDraft>,
    pub(crate) meta_beat: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoverageDraft {
    pub(crate) point_id: Option<String>,
    pub(crate) placement: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetectiveAiActDraft {
    pub(crate) id: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) narrative: Option<String>,
    pub(crate) seeds_meta: Option<bool>,
    /// Investigation acts: the evidence cards discovered in this beat.
    pub(crate) evidence: Option<Vec<DetectiveAiEvidenceDraft>>,
    /// Testimony acts: who testifies + their statements.
    pub(crate) witness_name: Option<String>,
    pub(crate) witness_role: Option<String>,
    pub(crate) testimony: Option<Vec<DetectiveAiTestimonyDraft>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetectiveAiTestimonyDraft {
    pub(crate) id: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) is_false: Option<bool>,
    pub(crate) key_evidence_id: Option<String>,
    #[serde(default)]
    pub(crate) highlights: Option<Vec<String>>,
    pub(crate) press_response: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetectiveAiEvidenceDraft {
    pub(crate) id: String,
    pub(crate) title: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) source_ref: Option<String>,
}
