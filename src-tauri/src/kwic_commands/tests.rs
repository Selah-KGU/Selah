use super::*;

#[test]
fn parses_kwic_information_list_rows() {
    let html = r#"
    <div id="information_list">
      <select id="informationType" name="informationType">
        <option value="10" selected="selected">呼び出し・重要なお知らせ</option>
        <option value="11">授業のお知らせ</option>
        <option value="12">その他</option>
      </select>
      <div class="contents-list">
        <div class="contents-display-flex-exchange-sp contents-display-flex-padding-sp result-list">
          <div class="portal-information-list-title sp-contents-hidden">
            <span id="title_1667108" class="link-txt break" data1="1667108" data2="02">6月3日（水）のシャトルバスの運行について</span>
            <span class="portal-information-priority portal-information-priority-urgency-color">NEW</span>
          </div>
          <div class="portal-information-list-date sp-contents-hidden">
            <span>2026/06/02 17:05</span>
            <span class="contents-time-to"></span>
            <span>2026/06/04 00:00</span>
          </div>
          <div class="portal-information-list-division sp-contents-hidden">学生課</div>
        </div>
        <div class="contents-display-flex-exchange-sp contents-display-flex-padding-sp result-list">
          <div class="portal-information-list-title sp-contents-hidden">
            <span id="title_1662480" class="link-txt break" data1="1662480" data2="04">【保健館より】尿の再検査が必要です</span>
          </div>
          <div class="portal-information-list-date sp-contents-hidden">
            <span>2026/05/28 09:00</span>
            <span class="contents-time-to"></span>
            <span>2026/06/30 00:00</span>
          </div>
          <div class="portal-information-list-division sp-contents-hidden">保健館</div>
        </div>
      </div>
    </div>
    "#;

    let document = scraper::Html::parse_document(html);
    let (section, items) = parse_information_list_items(&document, Some("10")).unwrap();

    assert_eq!(section, "呼出し・重要なお知らせ");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "1667108");
    assert_eq!(items[0].information_type, "10");
    assert_eq!(items[0].category_cd, "02");
    assert_eq!(items[0].date, "2026/06/02 17:05");
    assert_eq!(items[0].category, "学生課");
    assert_eq!(items[1].category_cd, "04");
}

#[test]
fn parses_kwic_other_information_list_rows() {
    let html = r#"
    <div id="information_list">
      <select id="informationType" name="informationType">
        <option value="10">呼び出し・重要なお知らせ</option>
        <option value="11">授業のお知らせ</option>
        <option value="12" selected="selected">その他</option>
      </select>
      <div class="contents-list">
        <div class="contents-display-flex-exchange-sp contents-display-flex-padding-sp result-list">
          <div class="portal-information-list-title sp-contents-hidden">
            <span id="title_1660000" class="link-txt break" data1="1660000" data2="0">その他のお知らせ</span>
          </div>
          <div class="portal-information-list-date sp-contents-hidden">
            <span>2026/06/03 10:00</span>
            <span class="contents-time-to"></span>
            <span>2026/06/30 00:00</span>
          </div>
          <div class="portal-information-list-division sp-contents-hidden">学生課</div>
        </div>
      </div>
    </div>
    "#;

    let document = scraper::Html::parse_document(html);
    let (section, items) = parse_information_list_items(&document, Some("12")).unwrap();

    assert_eq!(section, "その他");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].information_type, "12");
    assert_eq!(items[0].category_cd, "0");
}

#[test]
fn merges_information_list_without_duplicate_ids() {
    let html = r#"
    <div id="information_list">
      <select id="informationType" name="informationType">
        <option value="10" selected="selected">呼び出し・重要なお知らせ</option>
      </select>
      <div class="contents-list">
        <div class="contents-display-flex-exchange-sp contents-display-flex-padding-sp result-list">
          <div class="portal-information-list-title sp-contents-hidden">
            <span id="title_1667108" class="link-txt break" data1="1667108" data2="02">既存のお知らせ</span>
          </div>
          <div class="portal-information-list-date sp-contents-hidden"><span>2026/06/02 17:05</span></div>
          <div class="portal-information-list-division sp-contents-hidden">学生課</div>
        </div>
        <div class="contents-display-flex-exchange-sp contents-display-flex-padding-sp result-list">
          <div class="portal-information-list-title sp-contents-hidden">
            <span id="title_1667109" class="link-txt break" data1="1667109" data2="02">新しいお知らせ</span>
          </div>
          <div class="portal-information-list-date sp-contents-hidden"><span>2026/06/03 10:00</span></div>
          <div class="portal-information-list-division sp-contents-hidden">学生課</div>
        </div>
      </div>
    </div>
    "#;
    let mut sections = vec![KwicPortalSection {
        title: "呼出し・重要なお知らせ".to_string(),
        items: vec![KwicPortalItem {
            id: "1667108".to_string(),
            title: "既存のお知らせ".to_string(),
            date: "2026/06/02 17:05".to_string(),
            category: "学生課".to_string(),
            url: String::new(),
            important: false,
            information_type: "10".to_string(),
            person_category_cd: "0".to_string(),
            category_cd: "02".to_string(),
        }],
    }];

    let (parsed, added) = merge_information_list_sections(&mut sections, html, Some("10"));

    assert_eq!(parsed, 2);
    assert_eq!(added, 1);
    assert_eq!(sections[0].items.len(), 2);
    assert!(sections[0].items.iter().any(|item| item.id == "1667109"));
}

#[test]
fn skips_kwic_class_information_list_rows() {
    let html = r#"
    <div id="information_list">
      <select id="informationType" name="informationType">
        <option value="11" selected="selected">授業のお知らせ</option>
      </select>
      <div class="contents-list">
        <div class="contents-display-flex-exchange-sp contents-display-flex-padding-sp result-list">
          <div class="portal-information-list-title sp-contents-hidden">
            <span id="title_1667110" class="link-txt break" data1="1667110" data2="0">授業のお知らせ</span>
          </div>
          <div class="portal-information-list-date sp-contents-hidden"><span>2026/06/03 11:00</span></div>
          <div class="portal-information-list-division sp-contents-hidden">教務課</div>
        </div>
      </div>
    </div>
    "#;
    let mut sections = Vec::new();

    let (parsed, added) = merge_information_list_sections(&mut sections, html, Some("11"));

    assert_eq!(parsed, 0);
    assert_eq!(added, 0);
    assert!(sections.is_empty());
}

#[test]
fn parses_kwic_cabinet_reference_rows() {
    let html = r#"
    <div class="block block-area clearfix cabinetList">
      <div class="contents-list">
        <div class="result-list contents-display-flex result-data type-list" id="cabinetList_126">
          <input type="hidden" value="216" name="cabinetId" class="listCabinetId">
          <input type="hidden" value="教務機構" name="cabinetName" class="listCabinetName">
          <input type="hidden" value="1" name="cabinetLevel" class="listCabinetLevel">
          <input type="hidden" value="/cabinet/reference?typeCd=0" class="listUrl">
          <div class="cabinet-view-list-item">
            <div class="cabinet-view-list-name">
              <a class="cabinet-area-title-txt cabinetDisplayLink cabinet-title-omit-sp">教務機構</a>
            </div>
            <div class="cabinet-view-list-new">
              <span class="cabinet-area-new not-new" data-value="2026/05/19">NEW</span>
            </div>
            <div class="cabinet-view-list-createdate"><span>2026/05/19</span></div>
          </div>
        </div>
        <div class="result-list contents-display-flex result-data type-list" id="cabinetList_1415">
          <input type="hidden" value="264" name="cabinetId" class="listCabinetId">
          <input type="hidden" value="国際教育・協力センター（CIEC）: 海外への留学" name="cabinetName" class="listCabinetName">
          <input type="hidden" value="1" name="cabinetLevel" class="listCabinetLevel">
          <input type="hidden" value="/cabinet/reference?typeCd=0" class="listUrl">
          <div class="cabinet-view-list-item">
            <div class="cabinet-view-list-name"><a class="cabinetDisplayLink">国際教育・協力センター（CIEC）: 海外への留学</a></div>
            <div class="cabinet-view-list-new"><span class="cabinet-area-new" data-value="2026/05/27">NEW</span></div>
            <div class="cabinet-view-list-createdate"><span>2026/05/27</span></div>
          </div>
        </div>
      </div>
    </div>
    "#;

    let parsed = parse_cabinet_reference(html);

    assert_eq!(parsed.items.len(), 2);
    assert_eq!(parsed.items[0].cabinet_id, "216");
    assert_eq!(parsed.items[0].name, "教務機構");
    assert_eq!(parsed.items[0].updated_at, "2026/05/19");
    assert!(!parsed.items[0].is_new);
    assert_eq!(parsed.items[1].cabinet_id, "264");
    assert!(parsed.items[1].is_new);
    assert!(parsed.items[1].url.contains("/cabinet/reference?"));
    assert!(parsed.items[1].url.contains("typeCd=0"));
    assert!(parsed.items[1].url.contains("cabinetId=264"));
    assert!(parsed.items[1].url.contains("directLink=1"));
}

#[test]
fn compacts_oversized_inline_images() {
    let small = "data:image/png;base64,AAAA";
    let html = format!("<p><img src='{small}'></p>");
    assert!(compact_inline_images(&html).contains(small));

    let huge = "A".repeat(50 * 1024);
    let html = format!("<p><img src='data:image/png;base64,{huge}'></p>");
    let compacted = compact_inline_images(&html);
    assert!(!compacted.contains(&huge));
    assert!(compacted.len() < html.len() / 10);
}
