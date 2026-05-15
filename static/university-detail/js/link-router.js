function lunaItemMatchesLink(url, item, anchorText) {
  if (!item) return false;
  if (item.url && sameCoreUrl(url, item.url)) return true;
  if (item.url) {
    var itemUrl = resolveUniversityUrl(item.url);
    if (itemUrl) {
      var keys = ['reportId', 'surveyId', 'forumId', 'threadId', 'idnumber'];
      var sameKeys = false;
      for (var i = 0; i < keys.length; i++) {
        var key = keys[i];
        var av = url.searchParams.get(key) || '';
        var bv = itemUrl.searchParams.get(key) || '';
        if (av || bv) {
          if (av !== bv) return false;
          sameKeys = true;
        }
      }
      if (sameKeys && url.pathname === itemUrl.pathname) return true;
    }
  }
  return titlesLooselyMatch(anchorText, item.title);
}

function findLunaCourseItemMatch(course, url, anchorText) {
  if (!course) return null;
  var groups = [
    { kind: 'report', list: course.reports || [] },
    { kind: 'exam', list: course.examinations || [] },
    { kind: 'discussion', list: course.discussions || [] },
    { kind: 'survey', list: course.surveys || [] }
  ];
  for (var gi = 0; gi < groups.length; gi++) {
    var group = groups[gi];
    for (var ii = 0; ii < group.list.length; ii++) {
      var item = group.list[ii];
      if (lunaItemMatchesLink(url, item, anchorText)) {
        return {
          kind: group.kind,
          item: item,
          course_name: course.course_name || ''
        };
      }
    }
  }

  if ((url.pathname || '').indexOf('/lms/coursetop/information/listdetail') === 0) {
    var infoId = url.searchParams.get('informationId') || '';
    var announcements = course.announcements || [];
    for (var ai = 0; ai < announcements.length; ai++) {
      var ann = announcements[ai];
      if ((ann.info_id || '') === infoId || titlesLooselyMatch(anchorText, ann.title)) {
        return {
          kind: 'announcement',
          item: ann,
          course_name: course.course_name || ''
        };
      }
    }
  }

  return null;
}

async function findLunaTodoMatch(url, anchorText) {
  var todos = await readJsonCache('luna_todo');
  if (!Array.isArray(todos)) return null;
  for (var i = 0; i < todos.length; i++) {
    var todo = todos[i];
    if (todo.url && sameCoreUrl(url, todo.url)) return todo;
  }
  for (var j = 0; j < todos.length; j++) {
    var item = todos[j];
    if (titlesLooselyMatch(anchorText, item.content_name)) return item;
  }
  return null;
}

function findLunaExactCourseItem(course, url) {
  if (!course || !url) return null;

  var reportId = exactParam(url, 'reportId');
  if (reportId && Array.isArray(course.reports)) {
    for (var ri = 0; ri < course.reports.length; ri++) {
      var report = course.reports[ri];
      var reportUrl = resolveUniversityUrl(report.url || '');
      if (reportUrl && exactParam(reportUrl, 'reportId') === reportId) {
        return { kind: 'report', item: report, course_name: course.course_name || '' };
      }
    }
  }

  var examinationId = exactParam(url, 'examinationId');
  if (examinationId && Array.isArray(course.examinations)) {
    for (var ei = 0; ei < course.examinations.length; ei++) {
      var exam = course.examinations[ei];
      var examUrl = resolveUniversityUrl(exam.url || '');
      if (examUrl && exactParam(examUrl, 'examinationId') === examinationId) {
        return { kind: 'exam', item: exam, course_name: course.course_name || '' };
      }
    }
  }

  var surveyId = exactParam(url, 'surveyId');
  if (surveyId && Array.isArray(course.surveys)) {
    for (var si = 0; si < course.surveys.length; si++) {
      var survey = course.surveys[si];
      var surveyUrl = resolveUniversityUrl(survey.url || '');
      if (surveyUrl && exactParam(surveyUrl, 'surveyId') === surveyId) {
        return { kind: 'survey', item: survey, course_name: course.course_name || '' };
      }
    }
  }

  var forumId = exactParam(url, 'forumId');
  var threadId = exactParam(url, 'threadId');
  if (forumId && Array.isArray(course.discussions)) {
    for (var di = 0; di < course.discussions.length; di++) {
      var disc = course.discussions[di];
      var discUrl = resolveUniversityUrl(disc.url || '');
      if (!discUrl || exactParam(discUrl, 'forumId') !== forumId) continue;
      if (threadId && exactParam(discUrl, 'threadId') && exactParam(discUrl, 'threadId') !== threadId) continue;
      return {
        kind: exactParam(discUrl, 'threadId') ? 'thread' : 'discussion',
        item: disc,
        course_name: course.course_name || ''
      };
    }
  }

  if ((url.pathname || '').indexOf('/lms/coursetop/information/listdetail') === 0) {
    var infoId = exactParam(url, 'informationId');
    var announcements = course.announcements || [];
    for (var ai = 0; ai < announcements.length; ai++) {
      if ((announcements[ai].info_id || '') === infoId) {
        return { kind: 'announcement', item: announcements[ai], course_name: course.course_name || '' };
      }
    }
  }

  return null;
}

async function findLunaExactTodoMatch(url) {
  var todos = await readJsonCache('luna_todo');
  if (!Array.isArray(todos)) return null;

  var idnumber = exactParam(url, 'idnumber');
  var reportId = exactParam(url, 'reportId');
  var examinationId = exactParam(url, 'examinationId');
  var surveyId = exactParam(url, 'surveyId');
  var forumId = exactParam(url, 'forumId');
  var threadId = exactParam(url, 'threadId');

  for (var i = 0; i < todos.length; i++) {
    var todo = todos[i];
    var todoUrl = resolveUniversityUrl(todo.url || '');
    if (!todoUrl) continue;
    if (idnumber && exactParam(todoUrl, 'idnumber') && exactParam(todoUrl, 'idnumber') !== idnumber) continue;
    if (reportId && exactParam(todoUrl, 'reportId') === reportId) return todo;
    if (examinationId && exactParam(todoUrl, 'examinationId') === examinationId) return todo;
    if (surveyId && exactParam(todoUrl, 'surveyId') === surveyId) return todo;
    if (forumId && exactParam(todoUrl, 'forumId') === forumId) {
      if (!threadId || !exactParam(todoUrl, 'threadId') || exactParam(todoUrl, 'threadId') === threadId) return todo;
    }
  }

  return null;
}

function internalTargetLabel(target) {
  return String((target && target.title) || '').trim();
}

function looksLikeRawLinkLabel(text, href) {
  var rawText = String(text || '').trim();
  var rawHref = String(href || '').trim();
  if (!rawText) return true;
  if (rawText === rawHref) return true;
  var resolved = resolveUniversityUrl(rawHref);
  return !!resolved && rawText === resolved.toString();
}

async function hydrateInternalLinkLabels(root) {
  var scope = root || document;
  var anchors = Array.from(scope.querySelectorAll('a[href]'));
  var touched = 0;
  for (var i = 0; i < anchors.length; i++) {
    var a = anchors[i];
    if (!document.documentElement.contains(a)) continue;
    var href = a.getAttribute('href') || '';
    if (!href || !looksLikeRawLinkLabel(a.textContent, href)) continue;
    try {
      var target = await resolveUniversityLinkTarget(href, a.textContent.trim() || href);
      var label = internalTargetLabel(target);
      if (!label) continue;
      if (!a.getAttribute('title')) {
        var full = resolveUniversityUrl(href);
        a.setAttribute('title', full ? full.toString() : href);
      }
      a.textContent = label;
      touched++;
      if (touched % 4 === 0) await delay(0);
    } catch (e) {}
  }
}

async function buildReportFallbackData(path, currentTitle, currentCourseName) {
  var url = resolveUniversityUrl(path);
  if (!url) return null;

  var idnumber = exactParam(url, 'idnumber');
  var todo = await findLunaExactTodoMatch(url);
  var course = idnumber ? await readLunaCourseDetail(idnumber) : null;
  var courseMatch = findLunaExactCourseItem(course, url);
  var item = courseMatch && courseMatch.item ? courseMatch.item : null;

  var title = (item && item.title) || (todo && todo.content_name) || currentTitle || '';
  var courseName = (course && course.course_name) || (courseMatch && courseMatch.course_name) || (todo && todo.course_name) || currentCourseName || '';

  var meta = [];
  if (item && item.period) meta.push(['公開期間', item.period]);
  if (todo && todo.deadline) meta.push(['締切', todo.deadline]);
  if (item && item.status) meta.push(['状態', item.status]);
  else if (todo && todo.status) meta.push(['状態', todo.status]);
  if (todo && todo.content_type) meta.push(['種別', todo.content_type]);

  if (!title && !courseName && meta.length === 0) return null;

  meta.push(['詳細', '課題本文は取得できませんでしたが、この課題はローカルキャッシュから特定しました。']);

  return {
    title: title,
    course_name: courseName,
    sections: [],
    attachments: [],
    meta: meta
  };
}

async function resolveUniversityLinkTarget(href, anchorText) {
  var url = resolveUniversityUrl(href);
  if (!url) return null;

  var service = detectUniversityService(url);
  if (!service) return null;

  if (service === 'kwic') {
    if ((url.pathname || '').indexOf('/portal/home/information/detail') === 0) {
      var infoId = url.searchParams.get('informationId') || '';
      if (infoId) {
        var kwicItem = await readKwicItemByInfoId(infoId);
        if (kwicItem) {
          return {
            action: 'kwic_detail',
            title: kwicItem.title || anchorText || 'KWIC',
            information_id: kwicItem.id,
            information_type: kwicItem.information_type || '',
            person_category_cd: kwicItem.person_category_cd || '',
            category_cd: kwicItem.category_cd || ''
          };
        }
      }
    }
    return { action: 'browser', url: url.toString(), title: anchorText || 'KWIC' };
  }

  if (service === 'kgc') {
    var kgcEntry = await getKgcEntryByPath(url.pathname + url.search);
    if (!kgcEntry) kgcEntry = await getKgcEntryByPath(url.pathname);
    if (kgcEntry && kgcEntry.detail_path) {
      return {
        action: 'kgc_detail',
        path: kgcEntry.detail_path,
        title: kgcEntry.name || anchorText || '授業詳細'
      };
    }
    return { action: 'browser', url: url.toString(), title: anchorText || 'KG-Course' };
  }

  if (service === 'luna') {
    var idnumber = resolveLunaIdnumber(url);
    var currentCourseName = readCurrentParam('courseName') || _currentCourseName || '';

    if ((url.pathname || '') === '/lms/course' || (url.pathname || '') === '/lms/contents') {
      if (url.hash === '#attendance' && idnumber) {
        return {
          action: 'luna_attendance',
          idnumber: idnumber,
          title: anchorText || '出席',
          course_name: currentCourseName
        };
      }
      if (!idnumber) {
        return {
          action: 'browser',
          url: url.toString(),
          title: anchorText || currentCourseName || 'Luna'
        };
      }
      return {
        action: 'luna_course',
        idnumber: idnumber,
        title: anchorText || currentCourseName || '授業',
        course_name: currentCourseName,
        kgc_path: await findKgcPathByCourseName(currentCourseName)
      };
    }

    var course = idnumber ? await readLunaCourseDetail(idnumber) : null;
    var courseMatch = findLunaExactCourseItem(course, url) || findLunaCourseItemMatch(course, url, anchorText);

    if (courseMatch && courseMatch.kind === 'announcement' && idnumber) {
      return {
        action: 'luna_announcement',
        idnumber: idnumber,
        info_id: courseMatch.item.info_id || url.searchParams.get('informationId') || '',
        title: courseMatch.item.title || anchorText || 'お知らせ',
        course_name: courseMatch.course_name || currentCourseName
      };
    }

    if (courseMatch && courseMatch.kind === 'report') {
      return {
        action: 'luna_report',
        path: courseMatch.item.url || (url.pathname + url.search),
        idnumber: idnumber,
        report_id: url.searchParams.get('reportId') || '',
        title: courseMatch.item.title || anchorText || '課題',
        period: courseMatch.item.period || '',
        course_name: courseMatch.course_name || currentCourseName
      };
    }

    if (courseMatch && courseMatch.kind === 'discussion') {
      return {
        action: 'luna_discussion',
        path: courseMatch.item.url || (url.pathname + url.search),
        title: courseMatch.item.title || anchorText || '掲示板',
        course_name: courseMatch.course_name || currentCourseName
      };
    }

    if (courseMatch && courseMatch.kind === 'thread') {
      return {
        action: 'luna_thread',
        path: courseMatch.item.url || (url.pathname + url.search),
        title: courseMatch.item.title || anchorText || '掲示板スレッド',
        course_name: courseMatch.course_name || currentCourseName
      };
    }

    if (courseMatch && courseMatch.kind === 'survey') {
      return {
        action: 'luna_survey',
        path: courseMatch.item.url || (url.pathname + url.search),
        title: courseMatch.item.title || anchorText || 'アンケート',
        course_name: courseMatch.course_name || currentCourseName
      };
    }

    if (courseMatch && courseMatch.kind === 'exam') {
      var examUrl = resolveUniversityUrl(courseMatch.item.url || '') || url;
      return {
        action: 'browser',
        url: examUrl.toString(),
        title: courseMatch.item.title || anchorText || 'テスト'
      };
    }

    if ((url.pathname || '').indexOf('/lms/coursetop/information/listdetail') === 0 && idnumber) {
      return {
        action: 'luna_announcement',
        idnumber: idnumber,
        info_id: url.searchParams.get('informationId') || '',
        title: anchorText || 'お知らせ',
        course_name: currentCourseName
      };
    }

    if ((url.pathname || '').indexOf('/lms/course/report/submission') === 0) {
      var todoMatch = await findLunaExactTodoMatch(url) || await findLunaTodoMatch(url, anchorText);
      return {
        action: 'luna_report',
        path: url.pathname + url.search,
        idnumber: idnumber,
        report_id: url.searchParams.get('reportId') || '',
        title: (todoMatch && todoMatch.content_name) || anchorText || '課題',
        period: (todoMatch && todoMatch.deadline) || '',
        course_name: (todoMatch && todoMatch.course_name) || currentCourseName
      };
    }

    if ((url.pathname || '').indexOf('/lms/course/forums/thread') === 0) {
      return {
        action: 'luna_thread',
        path: url.pathname + url.search,
        title: anchorText || '掲示板スレッド',
        course_name: currentCourseName
      };
    }

    if ((url.pathname || '').indexOf('/lms/course/forums/themetop') === 0) {
      return {
        action: 'luna_discussion',
        path: url.pathname + url.search,
        title: anchorText || '掲示板',
        course_name: currentCourseName
      };
    }

    if ((url.pathname || '').indexOf('/lms/course/surveys') === 0) {
      return {
        action: 'luna_survey',
        path: url.pathname + url.search,
        title: anchorText || 'アンケート',
        course_name: currentCourseName
      };
    }

    return {
      action: 'browser',
      url: url.toString(),
      title: anchorText || currentCourseName || 'Luna'
    };
  }

  return null;
}

async function openResolvedUniversityLink(target) {
  var invoke = window.__TAURI__?.core?.invoke;
  if (!invoke || !target) return false;

  if (target.action === 'kwic_detail') {
    await invoke('kwic_open_detail_window', {
      title: target.title,
      informationId: target.information_id,
      informationType: target.information_type,
      personCategoryCd: target.person_category_cd,
      categoryCd: target.category_cd
    });
    return true;
  }

  if (target.action === 'kgc_detail') {
    await invoke('open_detail_window', {
      path: target.path,
      courseName: target.title
    });
    return true;
  }

  if (target.action === 'luna_course') {
    await invoke('university_open_detail_window', {
      path: '',
      title: target.title,
      mode: 'course',
      idnumber: target.idnumber,
      kgcPath: target.kgc_path || null,
      courseName: target.course_name || null
    });
    return true;
  }

  if (target.action === 'luna_attendance') {
    await invoke('university_open_detail_window', {
      path: '',
      title: target.title,
      mode: 'attendance',
      idnumber: target.idnumber,
      courseName: target.course_name || null
    });
    return true;
  }

  if (target.action === 'luna_announcement') {
    await invoke('university_open_detail_window', {
      path: '',
      title: target.title,
      mode: 'announcement',
      idnumber: target.idnumber,
      infoId: target.info_id,
      courseName: target.course_name || null
    });
    return true;
  }

  if (target.action === 'luna_report') {
    await invoke('university_open_detail_window', {
      path: target.path,
      title: target.title,
      mode: 'report',
      idnumber: target.idnumber || null,
      infoId: target.report_id || null,
      period: target.period || null,
      courseName: target.course_name || null
    });
    return true;
  }

  if (target.action === 'luna_discussion') {
    await invoke('university_open_detail_window', {
      path: target.path,
      title: target.title,
      mode: 'discussion',
      courseName: target.course_name || null
    });
    return true;
  }

  if (target.action === 'luna_thread') {
    await invoke('university_open_detail_window', {
      path: target.path,
      title: target.title,
      mode: 'thread',
      courseName: target.course_name || null
    });
    return true;
  }

  if (target.action === 'luna_survey') {
    await invoke('university_open_detail_window', {
      path: target.path,
      title: target.title,
      mode: 'survey',
      courseName: target.course_name || null
    });
    return true;
  }

  if (target.action === 'browser' && target.url) {
    await invoke('kwic_open_link', {
      url: target.url,
      title: target.title || target.url
    });
    return true;
  }

  return false;
}
