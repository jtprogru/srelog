//! Репозиторий заметок: поиск корня, записи смен, генерация INDEX.md и BACKLOG.md.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::md;
use crate::Res;

/// Шаблон, зашитый в бинарник: используется, если в репозитории нет oncall/TEMPLATE.md.
pub const BUILTIN_TEMPLATE: &str = include_str!("../template/entry.md");

const ENTRY_NAME_LEN: usize = "YYYY-MM-DD.md".len();

pub struct Entry {
    pub path: PathBuf,
    /// путь относительно oncall/, всегда через `/`
    pub rel: String,
}

pub struct Notes {
    pub root: PathBuf,
}

impl Notes {
    /// Корень: явный путь, затем `$SRELOG_ROOT`, затем поиск `oncall/` вверх от cwd.
    pub fn locate(explicit: Option<PathBuf>) -> Res<Self> {
        if let Some(p) = explicit {
            return Self::verify(p);
        }
        if let Ok(p) = env::var("SRELOG_ROOT") {
            if !p.is_empty() {
                return Self::verify(PathBuf::from(p));
            }
        }
        let cwd = env::current_dir().map_err(|e| format!("не читается cwd: {e}"))?;
        for dir in cwd.ancestors() {
            if dir.join("oncall").is_dir() {
                return Ok(Notes {
                    root: dir.to_path_buf(),
                });
            }
        }
        Err(format!(
            "не нашёл корень заметок: каталога oncall/ нет ни в {}, ни выше.\n\
             Укажи --root <path> или SRELOG_ROOT.",
            cwd.display()
        ))
    }

    fn verify(root: PathBuf) -> Res<Self> {
        if root.join("oncall").is_dir() {
            Ok(Notes { root })
        } else {
            Err(format!("в {} нет каталога oncall/", root.display()))
        }
    }

    pub fn oncall(&self) -> PathBuf {
        self.root.join("oncall")
    }

    pub fn entry_path(&self, date: &str) -> PathBuf {
        self.oncall().join(&date[..4]).join(format!("{date}.md"))
    }

    /// Шаблон из репозитория, иначе встроенный.
    pub fn template(&self) -> Res<String> {
        let p = self.oncall().join("TEMPLATE.md");
        match fs::read_to_string(&p) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BUILTIN_TEMPLATE.to_string()),
            Err(e) => Err(format!("не читается {}: {e}", p.display())),
        }
    }

    pub fn engineer(&self) -> String {
        if let Ok(v) = env::var("ONCALL_ENGINEER") {
            if !v.trim().is_empty() {
                return v.trim().to_string();
            }
        }
        let out = Command::new("git")
            .args(["-C", &self.root.to_string_lossy(), "config", "user.name"])
            .output();
        if let Ok(out) = out {
            if out.status.success() {
                let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !name.is_empty() {
                    return name;
                }
            }
        }
        "TBD".to_string()
    }

    /// Создаёт запись, если её ещё нет. Возвращает путь и признак «создали сейчас».
    pub fn ensure_entry(&self, date: &str) -> Res<(PathBuf, bool)> {
        let file = self.entry_path(date);
        if file.exists() {
            return Ok((file, false));
        }
        let body = self
            .template()?
            .replace("{{DATE}}", date)
            .replace("{{ENGINEER}}", &self.engineer());

        let dir = file.parent().expect("у записи всегда есть каталог года");
        fs::create_dir_all(dir).map_err(|e| format!("не создаётся {}: {e}", dir.display()))?;
        fs::write(&file, body).map_err(|e| format!("не пишется {}: {e}", file.display()))?;
        Ok((file, true))
    }

    /// Все записи `oncall/**/YYYY-MM-DD.md`, отсортированные по пути.
    pub fn entries(&self) -> Res<Vec<Entry>> {
        let base = self.oncall();
        let mut found = Vec::new();
        walk(&base, &mut found)?;
        found.sort();

        Ok(found
            .into_iter()
            .map(|path| {
                let rel = path
                    .strip_prefix(&base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                Entry { path, rel }
            })
            .collect())
    }

    pub fn write_index(&self) -> Res<PathBuf> {
        let mut entries = self.entries()?;
        entries.reverse(); // свежие сверху

        let mut out = String::from(
            "# Индекс дежурств\n\n\
             Сгенерировано `srelog index`. Правки вносить в сами записи.\n\n\
             | Дата | Дежурный | Контекст | Итог |\n\
             |------|----------|----------|------|\n",
        );

        for e in &entries {
            let text = read(&e.path)?;
            let date = md::front(&text, "date").unwrap_or_else(|| e.rel.clone());
            let eng = md::front(&text, "engineer").unwrap_or_else(|| "-".into());
            let ctx = md::front(&text, "context").unwrap_or_else(|| "-".into());
            let tldr = md::section_lead(&text, "Итог смены").unwrap_or_else(|| "-".into());
            out.push_str(&format!(
                "| [{}]({}) | {} | {} | {} |\n",
                date,
                e.rel,
                md::cell(&eng),
                md::cell(&ctx),
                md::cell(&tldr)
            ));
        }

        let p = self.oncall().join("INDEX.md");
        write(&p, &out)?;
        Ok(p)
    }

    pub fn write_backlog(&self) -> Res<PathBuf> {
        let mut rows: Vec<String> = Vec::new();
        for e in &self.entries()? {
            let text = read(&e.path)?;
            let date = md::front(&text, "date").unwrap_or_else(|| e.rel.clone());
            for row in md::backlog_rows(&text) {
                rows.push(format!("{row} | [{date}]({}) |", e.rel));
            }
        }
        rows.sort();

        let mut out = String::from(
            "# Кандидаты в бэклог из дежурств\n\n\
             Сгенерировано `srelog backlog` из секций «Кандидаты в бэклог». \
             Правки вносить в сами записи.\n\n\
             | Направление | Что завести | Основание | Дежурство |\n\
             |-------------|-------------|-----------|-----------|\n",
        );
        for r in rows {
            out.push_str(&r);
            out.push('\n');
        }

        let p = self.oncall().join("BACKLOG.md");
        write(&p, &out)?;
        Ok(p)
    }
}

// ---------------------------------------------------------------- файлы

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Res<()> {
    let rd = fs::read_dir(dir).map_err(|e| format!("не читается {}: {e}", dir.display()))?;
    for e in rd {
        let e = e.map_err(|e| format!("не читается {}: {e}", dir.display()))?;
        let path = e.path();
        let ft = e
            .file_type()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if ft.is_dir() {
            walk(&path, out)?;
        } else if is_entry_name(&e.file_name().to_string_lossy()) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_entry_name(name: &str) -> bool {
    name.len() == ENTRY_NAME_LEN && name.ends_with(".md") && check_date(&name[..10]).is_ok()
}

pub fn check_date(d: &str) -> Res<()> {
    let b = d.as_bytes();
    let ok = b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9]
            .iter()
            .all(|&i| b[i].is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(format!("дата должна быть YYYY-MM-DD, а не `{d}`"))
    }
}

pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn read(p: &Path) -> Res<String> {
    fs::read_to_string(p).map_err(|e| format!("не читается {}: {e}", p.display()))
}

pub fn write(p: &Path, body: &str) -> Res<()> {
    fs::write(p, body).map_err(|e| format!("не пишется {}: {e}", p.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_names() {
        assert!(check_date("2026-09-01").is_ok());
        assert!(check_date("2026-9-1").is_err());
        assert!(check_date("20260901").is_err());
        assert!(is_entry_name("2026-09-01.md"));
        assert!(!is_entry_name("TEMPLATE.md"));
        assert!(!is_entry_name("2026-09-01.txt"));
    }

    #[test]
    fn builtin_template_has_required_sections() {
        let t = BUILTIN_TEMPLATE;
        assert!(t.contains("{{DATE}}") && t.contains("{{ENGINEER}}"));
        for s in ["Итог смены", "Находки", "Кандидаты в бэклог"] {
            assert!(md::sections(t).iter().any(|x| x == s), "нет секции {s}");
        }
    }
}
