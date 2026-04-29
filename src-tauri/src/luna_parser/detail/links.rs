/// Classify a URL into a link type for display purposes.
pub(in crate::luna_parser) fn classify_link(url: &str, name: &str) -> String {
    let u = url.to_lowercase();
    let n = name.to_lowercase();

    // Internal Luna download paths -> file
    if !u.starts_with("http") {
        return "file".into();
    }

    if u.contains("zoom.us") || u.contains("zoom.") || u.contains("/lti/zoom") {
        return "zoom".into();
    }
    if u.contains("panopto") || u.contains("/lti/panopto") || u.contains("/Panopto/") {
        return "panopto".into();
    }
    if u.contains("youtube.com") || u.contains("youtu.be") || u.contains("vimeo.com") {
        return "video".into();
    }
    if u.contains("sharepoint.com") || u.contains("onedrive.live.com") || u.contains("1drv.ms") {
        return "cloud".into();
    }
    if u.contains("drive.google.com")
        || u.contains("docs.google.com")
        || u.contains("forms.gle")
        || u.contains("forms.google.com")
    {
        return "google".into();
    }
    if u.contains("teams.microsoft.com") || u.contains("teams.live.com") {
        return "teams".into();
    }

    let file_exts = [
        ".pdf", ".doc", ".docx", ".ppt", ".pptx", ".xls", ".xlsx", ".zip", ".rar", ".7z", ".mp4",
        ".mp3", ".wav", ".png", ".jpg", ".jpeg",
    ];
    for ext in &file_exts {
        if u.ends_with(ext) || n.ends_with(ext) {
            return "file".into();
        }
    }

    "web".into()
}
