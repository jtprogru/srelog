//! Настройки журнала: `srelog.toml` в корне заметок, рядом с `oncall/`.
//!
//! TOML разбирается вручную и ровно настолько, насколько нужно одному ключу:
//! верхнеуровневый `directions`, массив строк. Переносы внутри массива, комментарии
//! и оба вида кавычек понимаются; другие ключи и всё ниже первой таблицы `[...]` пропускаются.

/// Имя файла в корне заметок.
pub const FILE: &str = "srelog.toml";

/// Настройки, которые кладёт `srelog init`, если своих ещё нет.
pub const BUILTIN: &str = include_str!("../template/srelog.toml");

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// допустимые направления в порядке BACKLOG.md; пусто — направления не проверяются
    pub directions: Vec<String>,
}

impl Config {
    /// Место направления в BACKLOG.md: по списку, незнакомые после всех. Без списка место у всех одно.
    pub fn rank(&self, direction: &str) -> usize {
        self.directions
            .iter()
            .position(|d| d == direction)
            .unwrap_or(self.directions.len())
    }

    /// Направления нет в списке. Когда списка нет, незнакомых не бывает.
    pub fn unknown(&self, direction: &str) -> bool {
        !self.directions.is_empty() && !self.directions.iter().any(|d| d == direction)
    }
}

/// Разбирает `srelog.toml`. Ошибка — номер строки с единицы и что не так.
pub fn parse(text: &str) -> Result<Config, (usize, String)> {
    let mut directions = None;
    let mut lines = text.lines().zip(1..);

    while let Some((line, start)) = lines.next() {
        let t = line.trim_start();
        // таблица: ключи ниже уже не верхнего уровня
        if t.starts_with('[') {
            break;
        }
        let Some(value) = t
            .strip_prefix("directions")
            .and_then(|v| v.trim_start().strip_prefix('='))
        else {
            continue;
        };
        if directions.is_some() {
            return Err((start, "`directions` задан второй раз".into()));
        }

        // массив может тянуться на несколько строк: скармливаем их, пока не закроется
        let mut array = Array::default();
        let mut closed = array.feed(value).map_err(|e| (start, e))?;
        while !closed {
            let (line, n) = lines
                .next()
                .ok_or_else(|| (start, "массив `directions` не закрыт".to_string()))?;
            closed = array.feed(line).map_err(|e| (n, e))?;
        }
        directions = Some(array.items);
    }

    Ok(Config {
        directions: directions.unwrap_or_default(),
    })
}

/// Массив строк, который приходит построчно.
#[derive(Default)]
struct Array {
    items: Vec<String>,
    /// `[` уже была
    open: bool,
    /// после элемента ждём `,` или `]`
    comma: bool,
}

impl Array {
    /// Разбирает очередную строку файла. `Ok(true)` — массив закрылся.
    fn feed(&mut self, line: &str) -> Result<bool, String> {
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                ' ' | '\t' => {}
                '#' => break,
                '[' if !self.open => self.open = true,
                _ if !self.open => {
                    return Err(r#"`directions` ждёт массив строк: `["infra", "dev"]`"#.into());
                }
                ']' => {
                    let tail = chars.as_str().trim();
                    if !tail.is_empty() && !tail.starts_with('#') {
                        return Err(format!("лишнее после `]`: `{tail}`"));
                    }
                    return Ok(true);
                }
                ',' if self.comma => self.comma = false,
                _ if self.comma => {
                    return Err("между элементами `directions` нужна запятая".into());
                }
                ',' => return Err("лишняя запятая в `directions`".into()),
                '"' | '\'' => {
                    self.items.push(string(&mut chars, c)?);
                    self.comma = true;
                }
                _ => {
                    let word: String = std::iter::once(c)
                        .chain(
                            chars
                                .by_ref()
                                .take_while(|&ch| !matches!(ch, ',' | ']' | '#' | ' ' | '\t')),
                        )
                        .collect();
                    return Err(format!(
                        "`{word}` без кавычек: элементы `directions` — строки"
                    ));
                }
            }
        }
        Ok(false)
    }
}

/// Строка до закрывающей кавычки. В двойных кавычках понимает `\"` и `\\`, в одинарных экранирования нет.
fn string(chars: &mut std::str::Chars, quote: char) -> Result<String, String> {
    let mut out = String::new();
    while let Some(c) = chars.next() {
        match c {
            _ if c == quote => return Ok(out),
            '\\' if quote == '"' => match chars.next() {
                Some(e @ ('"' | '\\')) => out.push(e),
                Some(e) => return Err(format!("экранирование `\\{e}` не поддерживается")),
                None => break,
            },
            _ => out.push(c),
        }
    }
    Err("строка не закрыта".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs(text: &str) -> Vec<String> {
        parse(text).unwrap().directions
    }

    #[test]
    fn builtin_lists_directions() {
        assert!(
            !dirs(BUILTIN).is_empty(),
            "во встроенном srelog.toml нет направлений"
        );
    }

    #[test]
    fn one_line_array() {
        assert_eq!(dirs("directions = [\"infra\", 'dev']\n"), ["infra", "dev"]);
        assert_eq!(dirs("directions=[\"infra\"] # хвост\n"), ["infra"]);
    }

    #[test]
    fn multiline_array_with_comments() {
        let t = "\
# шапка
title = \"журнал\"

directions = [
  \"tooling\",  # утилиты дежурного
  # \"storage\",
  \"infra\",
]
";
        assert_eq!(dirs(t), ["tooling", "infra"]);
    }

    #[test]
    fn escapes_only_in_double_quotes() {
        assert_eq!(
            dirs(r#"directions = ["a\"b", "c\\d", 'e\f']"#),
            [r#"a"b"#, r"c\d", r"e\f"]
        );
    }

    #[test]
    fn no_list() {
        for t in [
            "",
            "# directions = [\"infra\"]\n",
            "directions = []\n",
            "directions_old = [\"infra\"]\n",
            // ключ из таблицы, а не верхнего уровня
            "[backlog]\ndirections = [\"infra\"]\n",
        ] {
            assert_eq!(parse(t).unwrap(), Config::default(), "{t:?}");
        }
    }

    #[test]
    fn errors_point_at_line() {
        let err = |t: &str| parse(t).unwrap_err();
        assert_eq!(
            err("directions = \"infra\"\n"),
            (
                1,
                r#"`directions` ждёт массив строк: `["infra", "dev"]`"#.into()
            )
        );
        assert_eq!(
            err("\ndirections = [infra, \"dev\"]\n"),
            (
                2,
                "`infra` без кавычек: элементы `directions` — строки".into()
            )
        );
        assert_eq!(
            err("directions = [\n  \"infra\"\n  \"dev\"\n]\n"),
            (3, "между элементами `directions` нужна запятая".into())
        );
        assert_eq!(
            err("directions = [\"infra\",,]\n"),
            (1, "лишняя запятая в `directions`".into())
        );
        // незакрытый массив показывает, где он начался
        assert_eq!(
            err("# шапка\ndirections = [\"infra\",\n\"dev\",\n"),
            (2, "массив `directions` не закрыт".into())
        );
        assert_eq!(
            err("directions = [\"infra]\n"),
            (1, "строка не закрыта".into())
        );
        assert_eq!(
            err("directions = [\"a\"] x\n"),
            (1, "лишнее после `]`: `x`".into())
        );
        assert_eq!(
            err("directions = [\"a\"]\ndirections = [\"b\"]\n"),
            (2, "`directions` задан второй раз".into())
        );
    }
}
