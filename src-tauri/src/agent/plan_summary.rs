//! Compact planner-facing summaries of tool JSON.
//!
//! The planner only needs identifiers and a short preview. Full tool
//! payloads stay out of the planning prompt.

use super::*;

pub(super) fn summarize_plan_tool_result(name: &str, json: &str) -> String {
    let parsed: Value = match if name == "computer_screenshot" {
        tool_result::screenshot_metadata(json)
    } else {
        serde_json::from_str(json)
    } {
        Ok(v) => v,
        Err(_) => return trim_to(json, 260),
    };
    let summary = match name {
        "list_recent_mail" => parsed.get("mails").and_then(|v| v.as_array()).map(|items| {
            items
                .iter()
                .take(3)
                .map(|m| {
                    format!(
                        "mail[id={}, subject={}]",
                        m.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                        m.get("subject").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        }),
        "list_luna_todos" => parsed.get("todos").and_then(|v| v.as_array()).map(|items| {
            items
                .iter()
                .take(3)
                .map(|t| {
                    format!(
                        "todo[title={}, course={}, luna_id={}, type={}, deadline={}]",
                        t.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("course").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("luna_id").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("type").and_then(|v| v.as_str()).unwrap_or(""),
                        t.get("deadline").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        }),
        "get_upcoming_deadlines" => {
            parsed
                .get("deadlines")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(3)
                        .map(|t| {
                            format!(
                                "deadline[title={}, deadline={}]",
                                t.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                t.get("deadline").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        }
        "list_downloaded_files" => parsed.get("files").and_then(|v| v.as_array()).map(|items| {
            items
                .iter()
                .take(3)
                .map(|f| {
                    format!(
                        "file[path={}, filename={}]",
                        f.get("path").and_then(|v| v.as_str()).unwrap_or(""),
                        f.get("filename").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        }),
        "get_course_context" => parsed.get("course").map(|course| {
            let name = course.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let materials = course
                .get("materials")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(2)
                        .map(|m| {
                            format!(
                                "material[title={}, url={}]",
                                m.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                m.get("url").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
                .unwrap_or_default();
            format!("course[name={}] {}", name, materials)
        }),
        "list_browser_windows" => parsed
            .get("windows")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .map(|w| {
                        format!(
                            "browser[target={}, type={}, title={}, url={}]",
                            w.get("target").and_then(|v| v.as_str()).unwrap_or(""),
                            w.get("type").and_then(|v| v.as_str()).unwrap_or(""),
                            w.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                            w.get("url").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
        "read_browser_page" => {
            let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let url = parsed.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let headings = parsed
                .get("headings")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(2)
                        .filter_map(|h| h.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default();
            Some(format!("page[title={}, url={}] {}", title, url, headings))
        }
        "computer_screenshot" => {
            let rect = parsed.get("screen_rect").unwrap_or(&Value::Null);
            let width = rect.get("width").and_then(|v| v.as_i64()).unwrap_or(0);
            let height = rect.get("height").and_then(|v| v.as_i64()).unwrap_or(0);
            let target = parsed.get("target").and_then(|v| v.as_str()).unwrap_or("");
            Some(format!(
                "screenshot[target={}, size={}x{}]",
                target, width, height
            ))
        }
        "browser_click"
        | "browser_mouse_click"
        | "browser_mouse_drag"
        | "computer_mouse_click"
        | "computer_mouse_drag"
        | "computer_scroll"
        | "browser_fill"
        | "browser_select_option"
        | "browser_press"
        | "browser_scroll"
        | "browser_wait_for" => {
            let action = parsed
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or(name);
            let url = parsed
                .get("current_url")
                .or_else(|| parsed.get("url"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let text = parsed
                .get("element")
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "action[name={}, text={}, url={}]",
                action, text, url
            ))
        }
        "open_browser_url" | "browser_back" | "browser_forward" | "browser_reload_page" => parsed
            .get("url")
            .and_then(|v| v.as_str())
            .map(|url| format!("browser[url={}]", url)),
        "open_copilot_page" => Some(format!(
            "copilot[page={}, title={}, target={}]",
            parsed.get("page").and_then(|v| v.as_str()).unwrap_or(""),
            parsed.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            parsed.get("target").and_then(|v| v.as_str()).unwrap_or(""),
        )),
        "search_notifications" | "list_recent_notifications" => parsed
            .get("notifications")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .map(|n| {
                        format!(
                            "notification[source={}, identifier={}, title={}]",
                            n.get("source").and_then(|v| v.as_str()).unwrap_or(""),
                            n.get("identifier").and_then(|v| v.as_str()).unwrap_or(""),
                            n.get("title").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
        "list_google_calendar_events" => {
            parsed
                .get("events")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(5)
                        .map(|e| {
                            format!(
                                "cal[id={}, title={}, date={} {}-{}]",
                                e.get("event_id").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("start_time").and_then(|v| v.as_str()).unwrap_or(""),
                                e.get("end_time").and_then(|v| v.as_str()).unwrap_or(""),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        }
        "create_google_calendar_event"
        | "delete_google_calendar_event"
        | "update_google_calendar_event" => parsed
            .get("message")
            .and_then(|v| v.as_str())
            .map(|s| format!("cal_action[{}]", s)),
        "get_today_brief" => {
            let class_count = parsed
                .get("classes")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let deadline_count = parsed
                .get("urgent_deadlines")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let first_class = parsed
                .get("classes")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
                .and_then(|c| c.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "today_brief[date={}, classes={}, urgent_deadlines={}, first={}]",
                parsed.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                class_count,
                deadline_count,
                first_class,
            ))
        }
        "get_weekly_summary" => {
            let week = parsed
                .get("current_week")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let preview = parsed
                .get("weekly_summary")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(60).collect::<String>())
                .unwrap_or_default();
            Some(format!(
                "weekly_summary[week={}, preview={}]",
                week, preview
            ))
        }
        "get_grades" => {
            let items = parsed
                .get("curriculum")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let deficit_count = items
                .iter()
                .filter(|c| c.get("deficit").and_then(|v| v.as_bool()).unwrap_or(false))
                .count();
            Some(format!(
                "grades[categories={}, deficits={}]",
                items.len(),
                deficit_count
            ))
        }
        "get_luna_activity_detail" => {
            let title = parsed
                .get("matched_title")
                .or_else(|| parsed.get("detail_title"))
                .or_else(|| parsed.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let luna_id = parsed
                .pointer("/source/luna_id")
                .or_else(|| parsed.get("luna_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let deadline = parsed
                .get("deadline")
                .or_else(|| parsed.get("period"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let attachments = parsed
                .get("attachments")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(5)
                        .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join(" / ")
                })
                .unwrap_or_default();
            let body_preview = parsed
                .get("body")
                .or_else(|| parsed.get("description"))
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(80).collect::<String>())
                .unwrap_or_default();
            Some(format!(
                "activity[title={}, luna_id={}, attachments={}, deadline={}, body_preview={}]",
                title, luna_id, attachments, deadline, body_preview
            ))
        }
        "list_luna_announcements" => {
            parsed
                .get("announcements")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(5)
                        .map(|a| {
                            format!(
                                "announce[title={}, luna_id={}, course={}, period={}]",
                                a.get("title").and_then(|v| v.as_str()).unwrap_or(""),
                                a.get("luna_id").and_then(|v| v.as_str()).unwrap_or(""),
                                a.get("course").and_then(|v| v.as_str()).unwrap_or(""),
                                a.get("period").and_then(|v| v.as_str()).unwrap_or(""),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        }
        "get_notification_detail" => {
            let title = parsed.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let source = parsed.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let body_preview = parsed
                .get("body")
                .or_else(|| parsed.get("body_html"))
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(120).collect::<String>())
                .unwrap_or_default();
            let attachment_count = parsed
                .get("attachments")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!(
                "notification_detail[source={}, title={}, attachments={}, body={}]",
                source, title, attachment_count, body_preview
            ))
        }
        "get_weather" => {
            let temp = parsed
                .get("current")
                .and_then(|c| c.get("temperature_c"))
                .and_then(|v| v.as_f64())
                .map(|t| format!("{}°C", t))
                .unwrap_or_default();
            let weather = parsed
                .get("current")
                .and_then(|c| c.get("weather"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!("weather[{} {}]", weather, temp))
        }
        "get_student_profile" => {
            let name = parsed.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let faculty = parsed.get("faculty").and_then(|v| v.as_str()).unwrap_or("");
            let dept = parsed
                .get("department")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            Some(format!(
                "profile[name={}, faculty={}, dept={}]",
                name, faculty, dept
            ))
        }
        "get_mail_profile" => {
            let name = parsed
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mail = parsed.get("mail").and_then(|v| v.as_str()).unwrap_or("");
            Some(format!("mail_profile[name={}, mail={}]", name, mail))
        }
        "list_syllabus_favorites" => {
            parsed
                .get("favorites")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .take(3)
                        .map(|f| {
                            format!(
                                "syllabus[{}]",
                                f.get("course_title").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        }
        "list_today_classes" | "list_week_classes" => parsed
            .get("classes")
            .and_then(|v| v.as_array())
            .map(|items| {
                let label = parsed
                    .get("day_of_week")
                    .or_else(|| parsed.get("week_label"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let classes: String = items
                    .iter()
                    .take(5)
                    .map(|c| {
                        format!(
                            "[{}{}]",
                            c.get("period").and_then(|v| v.as_str()).unwrap_or(""),
                            c.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("");
                format!("classes[{}] {}", label, classes)
            }),
        "get_cancellations" => {
            parsed
                .get("cancellations")
                .and_then(|v| v.as_array())
                .map(|items| {
                    let entries: String = items
                        .iter()
                        .take(3)
                        .map(|c| {
                            format!(
                                "[{} {}]",
                                c.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                                c.get("course_name").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    format!("cancellations[{}] {}", items.len(), entries)
                })
        }
        "get_makeup_classes" => {
            parsed
                .get("makeup_classes")
                .and_then(|v| v.as_array())
                .map(|items| {
                    let entries: String = items
                        .iter()
                        .take(3)
                        .map(|c| {
                            format!(
                                "[{} {}]",
                                c.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                                c.get("course_name").and_then(|v| v.as_str()).unwrap_or("")
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    format!("makeup_classes[{}] {}", items.len(), entries)
                })
        }
        "get_room_changes" => parsed
            .get("room_changes")
            .and_then(|v| v.as_array())
            .map(|items| {
                let entries: String = items
                    .iter()
                    .take(3)
                    .map(|c| {
                        format!(
                            "[{} {} → {}]",
                            c.get("date").and_then(|v| v.as_str()).unwrap_or(""),
                            c.get("course_name").and_then(|v| v.as_str()).unwrap_or(""),
                            c.get("room").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("");
                format!("room_changes[{}] {}", items.len(), entries)
            }),
        "get_exam_timetable" => parsed.get("exams").and_then(|v| v.as_array()).map(|items| {
            let entries: String = items
                .iter()
                .take(4)
                .map(|e| {
                    format!(
                        "[{} {}]",
                        e.get("day").and_then(|v| v.as_str()).unwrap_or(""),
                        e.get("course_name").and_then(|v| v.as_str()).unwrap_or("")
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            format!("exams[{}] {}", items.len(), entries)
        }),
        "get_registration" => {
            let year = parsed
                .get("year_semester")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let course_count = parsed
                .get("courses")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!(
                "registration[semester={}, courses={}]",
                year, course_count
            ))
        }
        "get_todo_guide" => {
            let age = parsed
                .get("generated_hours_ago")
                .and_then(|v| v.as_i64())
                .map(|h| format!("{}h ago", h))
                .unwrap_or_default();
            let priority = parsed
                .get("priority_summary")
                .and_then(|v| v.as_str())
                .map(|s| s.chars().take(80).collect::<String>())
                .unwrap_or_default();
            Some(format!(
                "todo_guide[generated={}, priority={}]",
                age, priority
            ))
        }
        "refresh_data" => {
            let refreshed = parsed
                .get("refreshed")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!("refresh_data[refreshed_count={}]", refreshed))
        }
        "search_courses" => parsed
            .get("matches")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .take(3)
                    .map(|m| {
                        format!(
                            "course[{}]",
                            m.get("display_name").and_then(|v| v.as_str()).unwrap_or("")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }),
        "get_course_detail" => {
            let code = parsed
                .get("kgc_code")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let plan_count = parsed
                .get("session_plan")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(format!(
                "course_detail[code={}, plan_sessions={}]",
                code, plan_count
            ))
        }
        _ => None,
    };
    trim_to(
        summary.as_deref().unwrap_or(json),
        CFG.plan_tool_result_chars,
    )
}
