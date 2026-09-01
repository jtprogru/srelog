//! Разбор и правка markdown записи: фронтматтер, секции, таблица бэклога,
//! нормализация заголовков во входящем тексте.

/// Что делать с заголовками в добавляемом тексте.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Headings {
    /// сдвинуть вглубь, чтобы они легли под секцию записи
    #[default]
    Shift,
    /// превратить в жирную строку
    Bold,
    /// выбросить строку заголовка
    Strip,
}

impl Headings {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "shift" => Ok(Headings::Shift),
            "bold" => Ok(Headings::Bold),
            "strip" => Ok(Headings::Strip),
            other => Err(format!(
                "--headings принимает shift, bold или strip, а не `{other}`"
            )),
        }
    }
}

// ---------------------------------------------------------------- примитивы

/// `## Заголовок` → `(2, "Заголовок")`. Отступ не допускается: это код, а не заголовок.
pub fn heading(line: &str) -> Option<(usize, &str)> {
    if !line.starts_with('#') {
        return None;
    }
    let n = line.chars().take_while(|c| *c == '#').count();
    if n > 6 {
        return None;
    }
    let rest = &line[n..];
    if rest.is_empty() {
        return Some((n, ""));
    }
    if !rest.starts_with(' ') {
        return None;
    }
    Some((n, rest.trim()))
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

/// Отрезает YAML-фронтматтер, если он есть. Незакрытый `---` фронтматтером не считается.
pub fn strip_frontmatter(text: &str) -> &str {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = match text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    {
        Some(r) => r,
        None => return text,
    };
    let mut off = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return &rest[off + line.len()..];
        }
        off += line.len();
    }
    text
}

// ---------------------------------------------------------------- чтение записи

/// Значение поля YAML-фронтматтера в шапке файла.
pub fn front(text: &str, key: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    let prefix = format!("{key}:");
    for line in lines {
        if line.trim() == "---" {
            return None;
        }
        if let Some(v) = line.strip_prefix(&prefix) {
            let v = v.trim();
            return if v.is_empty() {
                None
            } else {
                Some(v.to_string())
            };
        }
    }
    None
}

/// Первая содержательная строка секции; HTML-комментарии пропускаются.
pub fn section_lead(text: &str, name: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        if let Some(head) = line.strip_prefix("## ") {
            if inside {
                return None; // секция кончилась, ничего не нашли
            }
            inside = head.trim() == name;
            continue;
        }
        if !inside {
            continue;
        }
        let t = line.trim();
        if t.is_empty() || t.starts_with("<!--") {
            continue;
        }
        return Some(t.to_string());
    }
    None
}

/// Строки таблицы из секции «Кандидаты в бэклог», без шапки, разделителя и пустых.
pub fn backlog_rows(text: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut inside = false;

    for line in text.lines() {
        if line.starts_with("## ") {
            inside = line.contains("Кандидаты в бэклог");
            continue;
        }
        if !inside {
            continue;
        }
        let line = line.trim_end();
        if !line.starts_with('|') || line.contains("Направление") {
            continue;
        }
        // содержимое без разметки: пусто — строка-заглушка, одни дефисы — разделитель
        let body: String = line
            .chars()
            .filter(|c| *c != '|' && !c.is_whitespace())
            .collect();
        if body.is_empty() || body.chars().all(|c| c == '-' || c == ':') {
            continue;
        }
        rows.push(line.trim_end_matches(['|', ' ', '\t']).to_string());
    }
    rows
}

/// Заголовки секций второго уровня, в порядке следования.
pub fn sections(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut fence = false;
    for line in text.lines() {
        if is_fence(line) {
            fence = !fence;
            continue;
        }
        if fence {
            continue;
        }
        if let Some((2, t)) = heading(line) {
            out.push(t.to_string());
        }
    }
    out
}

/// Короткий запрос → точное имя секции. Понимает подстроки и латинские синонимы.
pub fn resolve_section(text: &str, query: &str) -> Result<String, String> {
    let all = sections(text);
    let needle = alias(query);
    let hits: Vec<&String> = all
        .iter()
        .filter(|s| s.to_lowercase().contains(&needle))
        .collect();

    match hits.len() {
        1 => Ok(hits[0].clone()),
        0 => Err(format!(
            "нет секции по запросу `{query}`. Есть: {}",
            all.join(", ")
        )),
        _ => Err(format!(
            "`{query}` подходит нескольким секциям: {}",
            hits.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn alias(query: &str) -> String {
    let q = query.to_lowercase();
    let mapped = match q.as_str() {
        "summary" | "итог" | "итоги" => "итог",
        "context" | "контекст" => "контекст",
        "findings" | "finding" | "находки" | "находка" => "находк",
        "artifacts" | "artifact" | "артефакты" | "артефакт" => "артефакт",
        "backlog" | "бэклог" | "беклог" | "кандидаты" => "кандидат",
        "questions" | "вопросы" | "вопрос" => "вопрос",
        other => other,
    };
    mapped.to_string()
}

// ---------------------------------------------------------------- правка записи

/// Дописывает блок в конец секции. Строку таблицы приклеивает к таблице без пустой строки.
pub fn append_to_section(text: &str, section: &str, block: &str) -> Result<String, String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    let mut start = None;
    let mut fence = false;
    for (i, line) in lines.iter().enumerate() {
        if is_fence(line) {
            fence = !fence;
            continue;
        }
        if !fence && matches!(heading(line), Some((2, t)) if t == section) {
            start = Some(i);
            break;
        }
    }
    let start = start.ok_or_else(|| format!("в записи нет секции «{section}»"))?;

    // конец секции — следующий заголовок второго уровня вне кода
    let mut end = start + 1;
    let mut fence = false;
    while end < lines.len() {
        if is_fence(&lines[end]) {
            fence = !fence;
        } else if !fence && matches!(heading(&lines[end]), Some((2, _))) {
            break;
        }
        end += 1;
    }

    // отступаем назад через хвостовые пустые строки секции
    let mut at = end;
    while at > start + 1 && lines[at - 1].trim().is_empty() {
        at -= 1;
    }

    let block_lines: Vec<String> = block.lines().map(str::to_string).collect();
    let starts_row = block_lines
        .iter()
        .find(|l| !l.trim().is_empty())
        .is_some_and(|l| l.starts_with('|'));
    let after_row = at > start + 1 && lines[at - 1].starts_with('|');

    let mut ins = Vec::new();
    if !(starts_row && after_row) {
        ins.push(String::new());
    }
    ins.extend(block_lines);
    if end < lines.len() {
        ins.push(String::new());
    }

    lines.splice(at..end, ins);
    let mut out = lines.join("\n");
    out.push('\n');
    Ok(out)
}

/// Готовит внешний текст к вставке: снимает фронтматтер и приводит заголовки к `base` и глубже.
pub fn normalize(input: &str, mode: Headings, base: usize) -> String {
    let body = strip_frontmatter(input);

    let mut min = usize::MAX;
    let mut fence = false;
    for line in body.lines() {
        if is_fence(line) {
            fence = !fence;
        } else if !fence {
            if let Some((n, _)) = heading(line) {
                min = min.min(n);
            }
        }
    }
    let delta = if min == usize::MAX {
        0
    } else {
        base.saturating_sub(min)
    };

    let mut out: Vec<String> = Vec::new();
    let mut fence = false;
    for line in body.lines() {
        if is_fence(line) {
            fence = !fence;
            out.push(line.to_string());
            continue;
        }
        if !fence {
            if let Some((n, title)) = heading(line) {
                match mode {
                    Headings::Shift => {
                        let lvl = (n + delta).min(6);
                        out.push(format!("{} {}", "#".repeat(lvl), title));
                    }
                    Headings::Bold => {
                        if !title.is_empty() {
                            out.push(format!("**{title}**"));
                        }
                    }
                    Headings::Strip => {}
                }
                continue;
            }
        }
        out.push(line.to_string());
    }

    collapse_blanks(&mut out);
    while out.first().is_some_and(|l| l.trim().is_empty()) {
        out.remove(0);
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Схлопывает подряд идущие пустые строки вне блоков кода.
fn collapse_blanks(lines: &mut Vec<String>) {
    let mut fence = false;
    let mut prev_blank = false;
    lines.retain(|l| {
        if is_fence(l) {
            fence = !fence;
            prev_blank = false;
            return true;
        }
        if fence {
            return true;
        }
        let blank = l.trim().is_empty();
        let drop = blank && prev_blank;
        prev_blank = blank;
        !drop
    });
}

/// Экранирует `|`, чтобы значение не развалило таблицу индекса.
pub fn cell(s: &str) -> String {
    s.replace('|', "\\|")
}

// ---------------------------------------------------------------- тесты

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
---
date: 2026-09-01
engineer: Mikhail Savin
context: [INC-42]
tags: [prod-cp-01]
---

# Дежурство 2026-09-01

## Итог смены

<!-- подсказка -->
Разбор шёл локальным kubectl.

## Кандидаты в бэклог

| Направление | Что завести | Основание (находка) |
|-------------|-------------|---------------------|
| tooling | L0-тул `quota_check(project_id)` | Бридж не ходит |
| access | Разрешить чтение квот | exec заблокирован |
|             |             |                     |

## Открытые вопросы

Ничего.
";

    const EMPTY: &str = "\
---
date: 2026-09-02
engineer: Кто-то
---

# Дежурство 2026-09-02

## Итог смены

## Находки

## Кандидаты в бэклог

| Направление | Что завести | Основание |
|-------------|-------------|-----------|

## Открытые вопросы
";

    #[test]
    fn frontmatter() {
        assert_eq!(front(SAMPLE, "date").as_deref(), Some("2026-09-01"));
        assert_eq!(front(SAMPLE, "engineer").as_deref(), Some("Mikhail Savin"));
        assert_eq!(front(SAMPLE, "context").as_deref(), Some("[INC-42]"));
        assert_eq!(front(SAMPLE, "нет"), None);
        assert_eq!(front("без шапки\ndate: 1", "date"), None);
    }

    #[test]
    fn lead_skips_comments() {
        assert_eq!(
            section_lead(SAMPLE, "Итог смены").as_deref(),
            Some("Разбор шёл локальным kubectl.")
        );
        assert_eq!(
            section_lead(SAMPLE, "Открытые вопросы").as_deref(),
            Some("Ничего.")
        );
        assert_eq!(section_lead(SAMPLE, "Находки"), None);
    }

    #[test]
    fn lead_absent_when_only_comment() {
        let t = "## Итог смены\n\n<!-- пусто -->\n\n## Дальше\n\nтекст\n";
        assert_eq!(section_lead(t, "Итог смены"), None);
    }

    #[test]
    fn backlog_drops_header_separator_and_blank() {
        assert_eq!(
            backlog_rows(SAMPLE),
            vec![
                "| tooling | L0-тул `quota_check(project_id)` | Бридж не ходит",
                "| access | Разрешить чтение квот | exec заблокирован",
            ]
        );
        assert!(backlog_rows(EMPTY).is_empty());
    }

    #[test]
    fn headings_parsed() {
        assert_eq!(heading("## Находки"), Some((2, "Находки")));
        assert_eq!(heading("#### Что-то  "), Some((4, "Что-то")));
        assert_eq!(heading("#нет пробела"), None);
        assert_eq!(heading("   ## отступ"), None);
        assert_eq!(heading("####### семь"), None);
        assert_eq!(heading("текст"), None);
    }

    #[test]
    fn frontmatter_stripped_only_when_closed() {
        assert_eq!(strip_frontmatter("---\na: 1\n---\nтело\n"), "тело\n");
        assert_eq!(strip_frontmatter("текст\n"), "текст\n");
        assert_eq!(strip_frontmatter("---\nне закрыт\n"), "---\nне закрыт\n");
    }

    #[test]
    fn section_lookup() {
        assert_eq!(resolve_section(EMPTY, "находки").unwrap(), "Находки");
        assert_eq!(resolve_section(EMPTY, "findings").unwrap(), "Находки");
        assert_eq!(
            resolve_section(EMPTY, "БЭКЛОГ").unwrap(),
            "Кандидаты в бэклог"
        );
        assert_eq!(
            resolve_section(EMPTY, "backlog").unwrap(),
            "Кандидаты в бэклог"
        );
        assert_eq!(resolve_section(EMPTY, "итог").unwrap(), "Итог смены");
        assert!(resolve_section(EMPTY, "неттакой").is_err());
    }

    #[test]
    fn normalize_shifts_headings_under_section() {
        let input = "# Верхний\n\nтекст\n\n## Вложенный\n\nещё\n";
        let got = normalize(input, Headings::Shift, 3);
        assert_eq!(got, "### Верхний\n\nтекст\n\n#### Вложенный\n\nещё");
    }

    #[test]
    fn normalize_never_promotes_deep_headings() {
        let input = "#### Уже глубоко\n\nтекст\n";
        assert_eq!(
            normalize(input, Headings::Shift, 3),
            "#### Уже глубоко\n\nтекст"
        );
    }

    #[test]
    fn normalize_clamps_at_six() {
        let input = "##### Пять\n\n###### Шесть\n";
        assert_eq!(
            normalize(input, Headings::Shift, 6),
            "###### Пять\n\n###### Шесть"
        );
    }

    #[test]
    fn normalize_bold_and_strip() {
        let input = "# Верхний\n\nтекст\n";
        assert_eq!(normalize(input, Headings::Bold, 3), "**Верхний**\n\nтекст");
        assert_eq!(normalize(input, Headings::Strip, 3), "текст");
    }

    #[test]
    fn normalize_leaves_code_alone() {
        let input = "# Заголовок\n\n```bash\n# это комментарий, не заголовок\ngrep -E 'a|b'\n```\n";
        let got = normalize(input, Headings::Shift, 3);
        assert!(got.contains("### Заголовок"));
        assert!(got.contains("# это комментарий, не заголовок"));
    }

    #[test]
    fn normalize_drops_frontmatter_and_edges() {
        let input = "---\ntitle: x\n---\n\n\nтекст\n\n\n";
        assert_eq!(normalize(input, Headings::Shift, 3), "текст");
    }

    #[test]
    fn append_into_empty_section() {
        let got = append_to_section(EMPTY, "Находки", "### Бридж\n\nне ходит").unwrap();
        assert!(got.contains("## Находки\n\n### Бридж\n\nне ходит\n\n## Кандидаты"));
    }

    #[test]
    fn append_into_last_section() {
        let got = append_to_section(EMPTY, "Открытые вопросы", "Ничего.").unwrap();
        assert!(got.ends_with("## Открытые вопросы\n\nНичего.\n"));
    }

    #[test]
    fn append_row_glues_to_table() {
        let got = append_to_section(EMPTY, "Кандидаты в бэклог", "| dev | X | Y |").unwrap();
        assert!(got.contains("|-------------|-------------|-----------|\n| dev | X | Y |\n"));
    }

    #[test]
    fn append_keeps_existing_content() {
        let once = append_to_section(EMPTY, "Находки", "первая").unwrap();
        let twice = append_to_section(&once, "Находки", "вторая").unwrap();
        assert!(twice.contains("## Находки\n\nпервая\n\nвторая\n\n## Кандидаты"));
    }

    #[test]
    fn append_reports_missing_section() {
        assert!(append_to_section(EMPTY, "Нет такой", "x").is_err());
    }

    #[test]
    fn pipes_escaped_in_cells() {
        assert_eq!(cell("a | b"), "a \\| b");
    }
}
