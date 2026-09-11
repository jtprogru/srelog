//! Репозиторий заметок: поиск корня, записи смен, генерация INDEX.md и BACKLOG.md.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::config::{self, Config};
use crate::md;
use crate::Res;

/// Шаблон, зашитый в бинарник: используется, если в репозитории нет oncall/TEMPLATE.md.
pub const BUILTIN_TEMPLATE: &str = include_str!("../template/entry.md");

/// README каталога заметок: кладётся при `srelog init`, если своего ещё нет.
pub const BUILTIN_README: &str = include_str!("../template/oncall-readme.md");

const ENTRY_NAME_LEN: usize = "YYYY-MM-DD.md".len();

pub struct Entry {
    pub path: PathBuf,
    /// путь относительно oncall/, всегда через `/`
    pub rel: String,
}

pub struct Notes {
    pub root: PathBuf,
}

/// Что `init` сделал на диске.
#[derive(Debug, Default)]
pub struct InitReport {
    /// файлы, которых не было
    pub created: Vec<PathBuf>,
    /// генерируемые файлы, содержимое которых изменилось
    pub rebuilt: Vec<PathBuf>,
    /// предупреждения сборки BACKLOG.md
    pub warnings: Vec<String>,
}

impl InitReport {
    /// На диске ничего не поменялось. Предупреждения не в счёт: файлов они не трогают.
    pub fn is_empty(&self) -> bool {
        self.created.is_empty() && self.rebuilt.is_empty()
    }
}

/// Собранный BACKLOG.md и сколько кандидатов легло в каждый раздел.
pub struct Backlog {
    pub text: String,
    /// без статуса, основная таблица
    pub open: usize,
    /// со статусом, раздел «Заведено и снято»
    pub closed: usize,
    /// незнакомые направления и сломанный srelog.toml; сборку они не останавливают
    pub warnings: Vec<String>,
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
             Укажи --root <path> или SRELOG_ROOT, либо заведи журнал здесь: srelog init",
            cwd.display()
        ))
    }

    fn verify(root: PathBuf) -> Res<Self> {
        if root.join("oncall").is_dir() {
            Ok(Notes { root })
        } else {
            Err(format!(
                "в {0} нет каталога oncall/\nЗаведи журнал: srelog init {0}",
                root.display()
            ))
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

    /// Настройки из `srelog.toml` в корне. Нет файла — нет и списка направлений.
    /// Нечитаемый или сломанный файл журнал не останавливает: настройки пустые, причина вторым значением.
    fn config(&self) -> (Config, Option<String>) {
        let p = self.root.join(config::FILE);
        let problem = match fs::read_to_string(&p) {
            Ok(text) => match config::parse(&text) {
                Ok(cfg) => return (cfg, None),
                Err((line, e)) => format!("{}:{line}: {e}", p.display()),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Config::default(), None),
            Err(e) => format!("не читается {}: {e}", p.display()),
        };
        (
            Config::default(),
            Some(format!("{problem}; направления не проверяю")),
        )
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

    /// Заводит `oncall/` со стартовым набором файлов. Существующее не перезаписывает.
    /// Возвращает корень и то, что появилось на диске.
    pub fn init(root: PathBuf) -> Res<(Self, InitReport)> {
        let oncall = root.join("oncall");
        let mut report = InitReport::default();
        if !oncall.is_dir() {
            fs::create_dir_all(&oncall)
                .map_err(|e| format!("не создаётся {}: {e}", oncall.display()))?;
            report.created.push(oncall.clone());
        }

        for (p, body) in [
            (oncall.join("TEMPLATE.md"), BUILTIN_TEMPLATE),
            (oncall.join("README.md"), BUILTIN_README),
            (root.join(config::FILE), config::BUILTIN),
        ] {
            if !p.exists() {
                write(&p, body)?;
                report.created.push(p);
            }
        }

        // генерируемые файлы собираются и на пустом наборе записей: журнал сразу консистентен
        let notes = Notes { root };
        let backlog = notes.render_backlog()?;
        report.warnings = backlog.warnings;
        for (name, body) in [
            ("INDEX.md", notes.render_index()?),
            ("BACKLOG.md", backlog.text),
        ] {
            let p = oncall.join(name);
            match fs::read_to_string(&p) {
                Ok(old) if old == body => {} // на диске уже ровно это, mtime не трогаем
                Ok(_) => {
                    write(&p, &body)?;
                    report.rebuilt.push(p);
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    write(&p, &body)?;
                    report.created.push(p);
                }
                Err(e) => return Err(format!("не читается {}: {e}", p.display())),
            }
        }

        Ok((notes, report))
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

    /// Содержимое INDEX.md: свежие дежурства сверху.
    pub fn render_index(&self) -> Res<String> {
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

        Ok(out)
    }

    pub fn write_index(&self) -> Res<PathBuf> {
        let p = self.oncall().join("INDEX.md");
        write(&p, &self.render_index()?)?;
        Ok(p)
    }

    /// Содержимое BACKLOG.md: открытые кандидаты из всех записей, под ними заведённые и снятые.
    /// В обеих таблицах направления идут в порядке srelog.toml, незнакомые — в конце и с предупреждением.
    pub fn render_backlog(&self) -> Res<Backlog> {
        let (cfg, problem) = self.config();
        let mut warnings: Vec<String> = problem.into_iter().collect();
        // ключ сортировки: место направления в списке, затем строка целиком
        let mut open: Vec<(usize, String)> = Vec::new();
        let mut closed: Vec<(usize, String)> = Vec::new();
        for e in &self.entries()? {
            let text = read(&e.path)?;
            let date = md::front(&text, "date").unwrap_or_else(|| e.rel.clone());
            for row in md::backlog_rows(&text) {
                if cfg.unknown(&row.direction) {
                    let what = if row.direction.is_empty() {
                        "направление не заполнено".to_string()
                    } else {
                        format!("направления `{}` нет в {}", row.direction, config::FILE)
                    };
                    warnings.push(format!("{}:{}: {what}", e.path.display(), row.line));
                }
                let line = (
                    cfg.rank(&row.direction),
                    format!("{} | [{date}]({}) |", row.text, e.rel),
                );
                if row.status.is_empty() {
                    open.push(line);
                } else {
                    closed.push(line);
                }
            }
        }
        open.sort();
        closed.sort();

        let mut out = String::from(
            "# Кандидаты в бэклог из дежурств\n\n\
             Сгенерировано `srelog backlog` из секций «Кандидаты в бэклог». \
             Правки вносить в сами записи.\n\n\
             | Направление | Что завести | Основание | Дежурство |\n\
             |-------------|-------------|-----------|-----------|\n",
        );
        for (_, r) in &open {
            out.push_str(r);
            out.push('\n');
        }

        // без статусов раздела нет, и журнал без четвёртой колонки собирается как раньше
        if !closed.is_empty() {
            out.push_str(
                "\n## Заведено и снято\n\n\
                 | Направление | Что завести | Основание | Статус | Дежурство |\n\
                 |-------------|-------------|-----------|--------|-----------|\n",
            );
            for (_, r) in &closed {
                out.push_str(r);
                out.push('\n');
            }
        }

        Ok(Backlog {
            text: out,
            open: open.len(),
            closed: closed.len(),
            warnings,
        })
    }

    pub fn write_backlog(&self) -> Res<(PathBuf, Backlog)> {
        let p = self.oncall().join("BACKLOG.md");
        let backlog = self.render_backlog()?;
        write(&p, &backlog.text)?;
        Ok((p, backlog))
    }
}

// ---------------------------------------------------------------- файлы

/// Абсолютный путь без `.` и `..`. Симлинки не разрешаются: путь может ещё не существовать.
pub fn absolute(p: &Path) -> Res<PathBuf> {
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|e| format!("не читается cwd: {e}"))?
            .join(p)
    };

    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// Журнал выше по дереву: заводить второй внутри первого почти всегда опечатка.
pub fn outer_notes(root: &Path) -> Option<PathBuf> {
    root.parent()?
        .ancestors()
        .find(|d| d.join("oncall").is_dir())
        .map(Path::to_path_buf)
}

/// Что говорит `SRELOG_ROOT` про только что заведённый журнал.
#[derive(Debug, PartialEq, Eq)]
pub enum RootEnv {
    /// Переменной нет: журнал найдётся только изнутри своего каталога.
    Unset,
    /// Переменная уже указывает сюда.
    Matches,
    /// Переменная уводит команды в другой журнал.
    Other(PathBuf),
}

pub fn root_env_state(root: &Path, env_root: Option<&str>) -> RootEnv {
    let raw = env_root.unwrap_or("").trim();
    if raw.is_empty() {
        return RootEnv::Unset;
    }
    match absolute(Path::new(raw)) {
        Ok(p) if p == root => RootEnv::Matches,
        Ok(p) => RootEnv::Other(p),
        Err(_) => RootEnv::Unset,
    }
}

/// Как закрепить путь к журналу в оболочке.
pub struct Persist {
    /// Файл профиля в виде, пригодном для показа: `~/.zshrc`.
    pub profile: String,
    /// Готовая команда: `echo 'export SRELOG_ROOT="..."' >> ~/.zshrc`.
    pub append: String,
    /// Та же установка для текущей сессии.
    pub session: String,
}

/// Профиль и команды под текущую оболочку. Чистая функция: `$SHELL` приходит снаружи.
pub fn persist_root(root: &Path, shell: Option<&str>) -> Persist {
    let name = shell.unwrap_or("").rsplit('/').next().unwrap_or("");
    let (profile, session) = match name {
        "zsh" => ("~/.zshrc", export_line(root, false)),
        "bash" if cfg!(target_os = "macos") => ("~/.bash_profile", export_line(root, false)),
        "bash" => ("~/.bashrc", export_line(root, false)),
        "fish" => ("~/.config/fish/config.fish", export_line(root, true)),
        _ => ("~/.profile", export_line(root, false)),
    };

    // путь журнала остаётся абсолютным: в двойных кавычках тильда не раскроется
    Persist {
        append: format!("echo '{}' >> {profile}", session.replace('\'', "'\\''")),
        profile: profile.to_string(),
        session,
    }
}

/// Строка установки переменной: `export VAR="..."` или, для fish, `set -gx VAR "..."`.
fn export_line(root: &Path, fish: bool) -> String {
    let mut quoted = String::from('"');
    for c in root.display().to_string().chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');

    if fish {
        format!("set -gx SRELOG_ROOT {quoted}")
    } else {
        format!("export SRELOG_ROOT={quoted}")
    }
}

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

    /// Свежий каталог во временной папке; имя уникально по тесту, чтобы гонок не было.
    fn temp_root(tag: &str) -> PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("srelog-test-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).expect("временный каталог");
        p
    }

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

    #[test]
    fn init_creates_layout() {
        let root = temp_root("init-layout");
        let (notes, report) = Notes::init(root.clone()).unwrap();

        assert_eq!(notes.root, root);
        for name in ["TEMPLATE.md", "README.md", "INDEX.md", "BACKLOG.md"] {
            let p = notes.oncall().join(name);
            assert!(p.is_file(), "нет {}", p.display());
            assert!(
                report.created.contains(&p),
                "{name} не отмечен как созданный"
            );
        }
        assert!(report.created.contains(&notes.oncall()));
        let cfg = notes.root.join("srelog.toml");
        assert!(cfg.is_file(), "нет {}", cfg.display());
        assert!(
            report.created.contains(&cfg),
            "srelog.toml не отмечен как созданный"
        );
        assert!(
            report.rebuilt.is_empty(),
            "на пустом месте нечего пересобирать"
        );
        // каталог года заводится только первой записью
        assert!(!notes.oncall().join("2026").exists());

        // после init корень находится обычным путём
        assert!(Notes::locate(Some(root.clone())).is_ok());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn init_keeps_existing_files() {
        let root = temp_root("init-idempotent");
        let (notes, _) = Notes::init(root.clone()).unwrap();

        let template = notes.oncall().join("TEMPLATE.md");
        write(&template, "## Своя секция\n").unwrap();
        let cfg = notes.root.join("srelog.toml");
        write(&cfg, "directions = [\"infra\"]\n").unwrap();

        let (_, report) = Notes::init(root.clone()).unwrap();
        assert!(
            report.is_empty(),
            "повторный init что-то тронул: {report:?}"
        );
        assert_eq!(read(&template).unwrap(), "## Своя секция\n");
        assert_eq!(read(&cfg).unwrap(), "directions = [\"infra\"]\n");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn init_reports_rebuilt_files() {
        let root = temp_root("init-rebuilt");
        let (notes, _) = Notes::init(root.clone()).unwrap();
        notes.ensure_entry("2026-09-02").unwrap();
        notes.write_index().unwrap();
        notes.write_backlog().unwrap();

        let index = notes.oncall().join("INDEX.md");
        write(&index, "# Индекс\n\nручная правка\n").unwrap();

        let (_, report) = Notes::init(root.clone()).unwrap();
        assert!(report.created.is_empty(), "лишнее создано: {report:?}");
        assert_eq!(report.rebuilt, vec![index.clone()], "отчёт не про то");
        assert!(read(&index).unwrap().contains("2026/2026-09-02.md"));

        // BACKLOG.md не менялся — в отчёт он не попал и остался нетронутым
        assert!(!report.rebuilt.contains(&notes.oncall().join("BACKLOG.md")));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn entry_lands_right_after_init() {
        let root = temp_root("init-then-add");
        let (notes, _) = Notes::init(root.clone()).unwrap();

        let (path, created) = notes.ensure_entry("2026-09-02").unwrap();
        assert!(created);
        assert_eq!(path, notes.oncall().join("2026").join("2026-09-02.md"));

        let text = read(&path).unwrap();
        assert!(text.contains("date: 2026-09-02"));
        assert!(!text.contains("{{"), "плейсхолдеры остались: {text}");

        notes.write_index().unwrap();
        assert!(read(&notes.oncall().join("INDEX.md"))
            .unwrap()
            .contains("2026/2026-09-02.md"));

        let _ = fs::remove_dir_all(&root);
    }

    /// Журнал после init со своим srelog.toml; `None` — без файла, как до списка направлений.
    fn journal(tag: &str, toml: Option<&str>) -> (PathBuf, Notes) {
        let root = temp_root(tag);
        let (notes, _) = Notes::init(root.clone()).unwrap();
        let p = root.join("srelog.toml");
        match toml {
            Some(text) => write(&p, text).unwrap(),
            None => fs::remove_file(&p).unwrap(),
        }
        (root, notes)
    }

    /// Запись с одной секцией бэклога: остальное BACKLOG.md не читает.
    fn put_backlog(notes: &Notes, date: &str, table: &str) {
        let p = notes.entry_path(date);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        write(
            &p,
            &format!("---\ndate: {date}\n---\n\n## Кандидаты в бэклог\n\n{table}"),
        )
        .unwrap();
    }

    const BACKLOG_HEAD: &str = "\
# Кандидаты в бэклог из дежурств

Сгенерировано `srelog backlog` из секций «Кандидаты в бэклог». Правки вносить в сами записи.

| Направление | Что завести | Основание | Дежурство |
|-------------|-------------|-----------|-----------|
";

    #[test]
    fn backlog_without_status_keeps_layout() {
        let (root, notes) = journal("backlog-plain", None);
        put_backlog(
            &notes,
            "2026-09-01",
            "\
| Направление | Что завести | Основание |
|-------------|-------------|-----------|
| tooling | Тул `quota_check(project_id)` | Бридж не видит квоты |
| infra   | Выровненная строка     | С пробелами              |
",
        );
        put_backlog(
            &notes,
            "2026-09-02",
            "\
| Направление | Что завести | Основание |
|-------------|-------------|-----------|
| access | Чтение квот | exec заблокирован |
",
        );

        let backlog = notes.render_backlog().unwrap();
        assert_eq!(
            backlog.text,
            format!(
                "{BACKLOG_HEAD}\
| access | Чтение квот | exec заблокирован | [2026-09-02](2026/2026-09-02.md) |
| infra   | Выровненная строка     | С пробелами | [2026-09-01](2026/2026-09-01.md) |
| tooling | Тул `quota_check(project_id)` | Бридж не видит квоты | [2026-09-01](2026/2026-09-01.md) |
"
            )
        );
        assert_eq!((backlog.open, backlog.closed), (3, 0));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn backlog_moves_rows_with_status_to_own_section() {
        let (root, notes) = journal("backlog-status", None);
        put_backlog(
            &notes,
            "2026-09-03",
            "\
| Направление | Что завести | Основание | Статус |
|-------------|-------------|-----------|--------|
| tooling | Тул `quota_check(project_id)` | Бридж не видит квоты | |
| infra | Обновить драйвер хранилища | Тихая порча данных | [PROJ-123](https://tracker.example/PROJ-123) |
| infra | Обновить драйвер хранилища | Рецидив | повтор 2026-09-03 |
",
        );

        let backlog = notes.render_backlog().unwrap();
        assert_eq!(
            backlog.text,
            format!(
                "{BACKLOG_HEAD}\
| tooling | Тул `quota_check(project_id)` | Бридж не видит квоты | [2026-09-03](2026/2026-09-03.md) |

## Заведено и снято

| Направление | Что завести | Основание | Статус | Дежурство |
|-------------|-------------|-----------|--------|-----------|
| infra | Обновить драйвер хранилища | Рецидив | повтор 2026-09-03 | [2026-09-03](2026/2026-09-03.md) |
| infra | Обновить драйвер хранилища | Тихая порча данных | [PROJ-123](https://tracker.example/PROJ-123) | [2026-09-03](2026/2026-09-03.md) |
"
            )
        );
        assert_eq!((backlog.open, backlog.closed), (1, 2));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn backlog_mixes_three_and_four_cells_in_one_table() {
        let (root, notes) = journal("backlog-mixed", None);
        put_backlog(
            &notes,
            "2026-09-04",
            "\
| Направление | Что завести | Основание |
|-------------|-------------|-----------|
| dev | Ретраи в клиенте | Таймауты на старте |
| infra | Дашборд квот | Смотрели руками | PROJ-7 |
| access | Чтение квот | exec заблокирован | |
",
        );

        let backlog = notes.render_backlog().unwrap();
        assert_eq!(
            backlog.text,
            format!(
                "{BACKLOG_HEAD}\
| access | Чтение квот | exec заблокирован | [2026-09-04](2026/2026-09-04.md) |
| dev | Ретраи в клиенте | Таймауты на старте | [2026-09-04](2026/2026-09-04.md) |

## Заведено и снято

| Направление | Что завести | Основание | Статус | Дежурство |
|-------------|-------------|-----------|--------|-----------|
| infra | Дашборд квот | Смотрели руками | PROJ-7 | [2026-09-04](2026/2026-09-04.md) |
"
            )
        );
        assert_eq!((backlog.open, backlog.closed), (2, 1));

        let _ = fs::remove_dir_all(&root);
    }

    /// Направления вразнобой, одно не заполнено.
    const MIXED_DIRECTIONS: &str = "\
| Направление | Что завести | Основание |
|-------------|-------------|-----------|
| storage | Обновить драйвер | Тихая порча данных |
| infra | Дашборд квот | Смотрели руками |
| | Без направления | Забыли заполнить |
| access | Чтение квот | exec заблокирован |
| dev | Ретраи в клиенте | Таймауты на старте |
";

    /// MIXED_DIRECTIONS за 2026-09-06 в порядке строк целиком, как без списка.
    const BY_LINE: &str = "\
| access | Чтение квот | exec заблокирован | [2026-09-06](2026/2026-09-06.md) |
| dev | Ретраи в клиенте | Таймауты на старте | [2026-09-06](2026/2026-09-06.md) |
| infra | Дашборд квот | Смотрели руками | [2026-09-06](2026/2026-09-06.md) |
| storage | Обновить драйвер | Тихая порча данных | [2026-09-06](2026/2026-09-06.md) |
| | Без направления | Забыли заполнить | [2026-09-06](2026/2026-09-06.md) |
";

    #[test]
    fn backlog_without_list_sorts_by_line_quietly() {
        // нет файла и нет ключа — одно и то же
        for (tag, toml) in [
            ("backlog-nolist-file", None),
            ("backlog-nolist-key", Some("# directions = [\"infra\"]\n")),
        ] {
            let (root, notes) = journal(tag, toml);
            put_backlog(&notes, "2026-09-06", MIXED_DIRECTIONS);

            let backlog = notes.render_backlog().unwrap();
            assert_eq!(backlog.text, format!("{BACKLOG_HEAD}{BY_LINE}"), "{tag}");
            assert!(backlog.warnings.is_empty(), "{tag}: {:?}", backlog.warnings);

            let _ = fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn backlog_follows_list_and_warns_about_unknown() {
        let (root, notes) = journal(
            "backlog-list",
            Some("directions = [\"infra\", \"access\"]\n"),
        );
        put_backlog(&notes, "2026-09-06", MIXED_DIRECTIONS);

        let backlog = notes.render_backlog().unwrap();
        assert_eq!(
            backlog.text,
            format!(
                "{BACKLOG_HEAD}\
| infra | Дашборд квот | Смотрели руками | [2026-09-06](2026/2026-09-06.md) |
| access | Чтение квот | exec заблокирован | [2026-09-06](2026/2026-09-06.md) |
| dev | Ретраи в клиенте | Таймауты на старте | [2026-09-06](2026/2026-09-06.md) |
| storage | Обновить драйвер | Тихая порча данных | [2026-09-06](2026/2026-09-06.md) |
| | Без направления | Забыли заполнить | [2026-09-06](2026/2026-09-06.md) |
"
            )
        );
        let entry = notes.entry_path("2026-09-06");
        assert_eq!(
            backlog.warnings,
            vec![
                format!(
                    "{}:9: направления `storage` нет в srelog.toml",
                    entry.display()
                ),
                format!("{}:11: направление не заполнено", entry.display()),
                format!(
                    "{}:13: направления `dev` нет в srelog.toml",
                    entry.display()
                ),
            ]
        );
        assert_eq!((backlog.open, backlog.closed), (5, 0));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn backlog_orders_closed_section_by_list_too() {
        let (root, notes) = journal(
            "backlog-list-closed",
            Some("directions = [\"tooling\", \"infra\"]\n"),
        );
        put_backlog(
            &notes,
            "2026-09-07",
            "\
| Направление | Что завести | Основание | Статус |
|-------------|-------------|-----------|--------|
| infra | Обновить драйвер хранилища | Тихая порча данных | PROJ-123 |
| tooling | Фильтр логов | Grep молчит | снято |
| tooling | Тул `quota_check` | Бридж не видит квоты | |
",
        );

        let backlog = notes.render_backlog().unwrap();
        assert_eq!(
            backlog.text,
            format!(
                "{BACKLOG_HEAD}\
| tooling | Тул `quota_check` | Бридж не видит квоты | [2026-09-07](2026/2026-09-07.md) |

## Заведено и снято

| Направление | Что завести | Основание | Статус | Дежурство |
|-------------|-------------|-----------|--------|-----------|
| tooling | Фильтр логов | Grep молчит | снято | [2026-09-07](2026/2026-09-07.md) |
| infra | Обновить драйвер хранилища | Тихая порча данных | PROJ-123 | [2026-09-07](2026/2026-09-07.md) |
"
            )
        );
        assert!(backlog.warnings.is_empty(), "{:?}", backlog.warnings);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn broken_config_warns_and_skips_checks() {
        let (root, notes) = journal("backlog-broken", Some("directions = [\"infra\", access]\n"));
        put_backlog(&notes, "2026-09-06", MIXED_DIRECTIONS);

        let backlog = notes.render_backlog().unwrap();
        assert_eq!(backlog.text, format!("{BACKLOG_HEAD}{BY_LINE}"));
        assert_eq!(
            backlog.warnings,
            vec![format!(
                "{}:1: `access` без кавычек: элементы `directions` — строки; направления не проверяю",
                root.join("srelog.toml").display()
            )]
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn outer_notes_spots_nesting() {
        let root = temp_root("nesting");
        Notes::init(root.clone()).unwrap();

        let inner = root.join("sub").join("deeper");
        fs::create_dir_all(&inner).unwrap();
        assert_eq!(outer_notes(&inner), Some(root.clone()));
        assert_eq!(outer_notes(&root), None, "свой же oncall/ — не вложенность");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn env_state_reads_variable() {
        let root = PathBuf::from("/notes/new");
        assert_eq!(root_env_state(&root, None), RootEnv::Unset);
        assert_eq!(root_env_state(&root, Some("")), RootEnv::Unset);
        assert_eq!(root_env_state(&root, Some("  ")), RootEnv::Unset);
        assert_eq!(root_env_state(&root, Some("/notes/new")), RootEnv::Matches);
        assert_eq!(root_env_state(&root, Some("/notes/new/")), RootEnv::Matches);
        assert_eq!(
            root_env_state(&root, Some("/notes/../notes/new")),
            RootEnv::Matches
        );
        assert_eq!(
            root_env_state(&root, Some("/notes/old")),
            RootEnv::Other(PathBuf::from("/notes/old"))
        );
    }

    #[test]
    fn persist_picks_profile_by_shell() {
        let root = PathBuf::from("/notes/new");

        let zsh = persist_root(&root, Some("/bin/zsh"));
        assert_eq!(zsh.profile, "~/.zshrc");
        assert_eq!(zsh.session, r#"export SRELOG_ROOT="/notes/new""#);
        assert_eq!(
            zsh.append,
            r#"echo 'export SRELOG_ROOT="/notes/new"' >> ~/.zshrc"#
        );

        let bash = persist_root(&root, Some("/opt/homebrew/bin/bash"));
        let expected = if cfg!(target_os = "macos") {
            "~/.bash_profile"
        } else {
            "~/.bashrc"
        };
        assert_eq!(bash.profile, expected);
        assert!(bash.append.ends_with(expected));

        let fish = persist_root(&root, Some("/usr/local/bin/fish"));
        assert_eq!(fish.profile, "~/.config/fish/config.fish");
        assert_eq!(fish.session, r#"set -gx SRELOG_ROOT "/notes/new""#);

        // неизвестная оболочка и пустой $SHELL — общий профиль
        assert_eq!(persist_root(&root, Some("/bin/ksh")).profile, "~/.profile");
        assert_eq!(persist_root(&root, None).profile, "~/.profile");
    }

    #[test]
    fn persist_quotes_awkward_paths() {
        let spaced = persist_root(Path::new("/notes/on call"), Some("/bin/zsh"));
        assert_eq!(spaced.session, r#"export SRELOG_ROOT="/notes/on call""#);

        let quoted = persist_root(Path::new("/notes/it's $HOME"), Some("/bin/zsh"));
        assert_eq!(quoted.session, r#"export SRELOG_ROOT="/notes/it's \$HOME""#);
        // одинарная кавычка не рвёт строку echo
        assert_eq!(
            quoted.append,
            r#"echo 'export SRELOG_ROOT="/notes/it'\''s \$HOME"' >> ~/.zshrc"#
        );
    }

    #[test]
    fn absolute_cleans_path() {
        assert_eq!(
            absolute(Path::new("/a/./b/../c")).unwrap(),
            PathBuf::from("/a/c")
        );
        let rel = absolute(Path::new("notes")).unwrap();
        assert!(rel.is_absolute() && rel.ends_with("notes"));
    }
}
