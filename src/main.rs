//! srelog — лог дежурств одной командой.
//!
//!   srelog add [СЕКЦИЯ] < файл    дописать в запись за сегодня и пересобрать индексы
//!   srelog init [ПУТЬ]            завести каталог oncall/ под журнал
//!   srelog shift [ДАТА]           создать пустую запись
//!   srelog index | backlog | sync пересобрать генерируемые файлы

mod md;
mod notes;

use std::io::Read;
use std::path::PathBuf;

use md::Headings;
use notes::Notes;

pub type Res<T> = Result<T, String>;

const USAGE: &str = "\
srelog — лог дежурств

  srelog add [СЕКЦИЯ]         дописать в запись то, что пришло на stdin,
                              и пересобрать INDEX.md с BACKLOG.md
  srelog init [ПУТЬ]          завести журнал: oncall/ с шаблоном и README
                              (по умолчанию — в текущем каталоге)
  srelog shift [ДАТА]         создать пустую запись из шаблона
  srelog index                пересобрать oncall/INDEX.md
  srelog backlog              пересобрать oncall/BACKLOG.md
  srelog sync                 пересобрать оба файла
  srelog sections             показать секции сегодняшней записи
  srelog template             напечатать встроенный шаблон

Секция по умолчанию — «Находки». Годится часть имени или латинский синоним:
находки/findings, итог/summary, контекст/context, артефакты/artifacts,
бэклог/backlog, вопросы/questions.

Опции add:
  -f, --file <path>     читать из файла, а не со stdin; можно несколько раз
  -d, --date <ДАТА>     в запись за другой день (по умолчанию сегодня)
  -t, --title <текст>   завернуть добавляемое в подзаголовок
      --headings <как>  что делать с заголовками во входе:
                        shift (по умолчанию) — сдвинуть под секцию,
                        bold — сделать жирной строкой, strip — выбросить
      --no-sync         не пересобирать INDEX.md и BACKLOG.md

Общие опции:
  --root <path>   корень заметок (каталог, в котором лежит oncall/)
                  по умолчанию: $SRELOG_ROOT, иначе поиск oncall/ вверх от cwd

Окружение:
  SRELOG_ROOT       корень заметок
  ONCALL_ENGINEER   имя дежурного; иначе git config user.name, иначе TBD

Примеры:
  srelog add <<'EOF'
  ### Бридж не ходит на prod-cp-01
  Весь разбор шёл локальным kubectl.
  EOF

  srelog add находки < ~/notes/raw.md
  srelog add бэклог <<< '| tooling | L0-тул quota_check | Бридж не ходит |'
  echo 'Разбор INC-42 шёл локальным kubectl.' | srelog add итог";

fn main() {
    if let Err(e) = run() {
        eprintln!("srelog: {e}");
        std::process::exit(1);
    }
}

fn run() -> Res<()> {
    let cli = Cli::parse(std::env::args().skip(1))?;

    match cli.cmd.as_deref() {
        None | Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            Ok(())
        }
        Some("template") => {
            print!("{}", notes::BUILTIN_TEMPLATE);
            Ok(())
        }
        Some("init") => cmd_init(&cli),
        Some("add") => cmd_add(&cli),
        Some("shift") => cmd_shift(&cli),
        Some("index") => {
            let n = Notes::locate(cli.root.clone())?;
            println!("{}", n.write_index()?.display());
            Ok(())
        }
        Some("backlog") => {
            let n = Notes::locate(cli.root.clone())?;
            println!("{}", n.write_backlog()?.display());
            Ok(())
        }
        Some("sync") => {
            let n = Notes::locate(cli.root.clone())?;
            println!("{}", n.write_index()?.display());
            println!("{}", n.write_backlog()?.display());
            Ok(())
        }
        Some("sections") => cmd_sections(&cli),
        Some(other) => Err(format!("неизвестная команда `{other}`\n\n{USAGE}")),
    }
}

// ---------------------------------------------------------------- команды

fn cmd_init(cli: &Cli) -> Res<()> {
    let root = match cli.args.first() {
        Some(p) => PathBuf::from(p),
        None => match &cli.root {
            Some(p) => p.clone(),
            None => std::env::current_dir().map_err(|e| format!("не читается cwd: {e}"))?,
        },
    };
    let root = notes::absolute(&root)?;

    if let Some(outer) = notes::outer_notes(&root) {
        eprintln!(
            "srelog: осторожно, выше уже есть журнал: {}/oncall",
            outer.display()
        );
    }

    let (notes, report) = Notes::init(root)?;
    if report.is_empty() {
        eprintln!("журнал уже заведён, ничего не менял");
    } else {
        for p in &report.created {
            eprintln!("создано: {}", p.display());
        }
        // INDEX.md и BACKLOG.md собираются из записей, ручные правки в них не переживают init
        for p in &report.rebuilt {
            eprintln!("пересобрано: {}", p.display());
        }
    }

    // предупреждения и подсказки не меняют код возврата: это подсказки, а не отказ работать
    let persist = notes::persist_root(&notes.root, std::env::var("SHELL").ok().as_deref());
    match notes::root_env_state(&notes.root, std::env::var("SRELOG_ROOT").ok().as_deref()) {
        notes::RootEnv::Matches => {
            eprintln!("SRELOG_ROOT уже указывает сюда — журнал доступен из любого каталога");
        }
        notes::RootEnv::Unset => {
            eprintln!();
            eprintln!("srelog ищет журнал через SRELOG_ROOT; без неё он найдётся только изнутри этого каталога.");
            eprintln!("Закрепи путь в профиле оболочки:");
            eprintln!();
            eprintln!("  {}", persist.append);
            eprintln!();
            eprintln!("и в текущей сессии:");
            eprintln!();
            eprintln!("  {}", persist.session);
        }
        notes::RootEnv::Other(old) => {
            eprintln!();
            eprintln!(
                "srelog: SRELOG_ROOT указывает на {} — команды пойдут туда, а не в этот журнал.",
                old.display()
            );
            eprintln!("Перенастрой на новый путь:");
            eprintln!();
            eprintln!("  {}", persist.append);
            eprintln!("  {}", persist.session);
            eprintln!();
            eprintln!(
                "Старую строку из {} потом убери, иначе она так и будет висеть выше.",
                persist.profile
            );
        }
    }

    println!("{}", notes.root.display());
    Ok(())
}

fn cmd_add(cli: &Cli) -> Res<()> {
    let notes = Notes::locate(cli.root.clone())?;
    let date = cli.date()?;

    // секцию проверяем до создания записи, чтобы опечатка не оставляла пустой файл
    let query = cli.args.first().map(String::as_str).unwrap_or("находки");
    let path = notes.entry_path(&date);
    let section = md::resolve_section(
        &if path.exists() {
            notes::read(&path)?
        } else {
            notes.template()?
        },
        query,
    )?;

    let raw = collect_input(&cli.files)?;
    if raw.trim().is_empty() {
        return Err("пустой ввод, нечего добавлять".into());
    }

    let (path, created) = notes.ensure_entry(&date)?;
    let text = notes::read(&path)?;

    // заголовок от --title занимает третий уровень, тогда вложенное начинается с четвёртого
    let base = if cli.title.is_some() { 4 } else { 3 };
    let body = md::normalize(&raw, cli.headings, base);
    let block = match &cli.title {
        Some(t) if body.is_empty() => format!("### {t}"),
        Some(t) => format!("### {t}\n\n{body}"),
        None => body,
    };

    let updated = md::append_to_section(&text, &section, &block)?;
    notes::write(&path, &updated)?;

    if created {
        eprintln!("завёл запись на {date}");
    }
    eprintln!("добавлено в «{section}»");
    println!("{}", path.display());

    if !cli.no_sync {
        notes.write_index()?;
        notes.write_backlog()?;
    }
    Ok(())
}

fn cmd_shift(cli: &Cli) -> Res<()> {
    let notes = Notes::locate(cli.root.clone())?;
    let date = match cli.args.first() {
        Some(d) => {
            notes::check_date(d)?;
            d.clone()
        }
        None => cli.date()?,
    };

    let (path, created) = notes.ensure_entry(&date)?;
    if !created {
        eprintln!("запись уже есть, ничего не переписываю");
    }
    println!("{}", path.display());

    if created && !cli.no_sync {
        notes.write_index()?;
    }
    Ok(())
}

fn cmd_sections(cli: &Cli) -> Res<()> {
    let notes = Notes::locate(cli.root.clone())?;
    let date = cli.date()?;
    let path = notes.entry_path(&date);
    let text = if path.exists() {
        notes::read(&path)?
    } else {
        notes.template()?
    };
    for s in md::sections(&text) {
        println!("{s}");
    }
    Ok(())
}

fn collect_input(files: &[PathBuf]) -> Res<String> {
    if files.is_empty() {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("не читается stdin: {e}"))?;
        return Ok(buf);
    }
    let mut parts = Vec::new();
    for f in files {
        parts.push(notes::read(f)?);
    }
    Ok(parts.join("\n\n"))
}

// ---------------------------------------------------------------- разбор аргументов

#[derive(Default)]
struct Cli {
    cmd: Option<String>,
    args: Vec<String>,
    root: Option<PathBuf>,
    files: Vec<PathBuf>,
    date_opt: Option<String>,
    title: Option<String>,
    headings: Headings,
    no_sync: bool,
}

impl Cli {
    fn parse(raw: impl Iterator<Item = String>) -> Res<Self> {
        let raw: Vec<String> = raw.collect();
        let mut cli = Cli::default();
        let mut positional = Vec::new();
        let mut i = 0;

        while i < raw.len() {
            let a = raw[i].clone();
            let (name, inline) = match a.split_once('=') {
                Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
                _ => (a.clone(), None),
            };

            let value = |cli_i: &mut usize| -> Res<String> {
                match &inline {
                    Some(v) => Ok(v.clone()),
                    None => {
                        *cli_i += 1;
                        raw.get(*cli_i)
                            .cloned()
                            .ok_or_else(|| format!("{name} требует значение"))
                    }
                }
            };

            match a.split('=').next().unwrap_or(&a) {
                "--root" => cli.root = Some(PathBuf::from(value(&mut i)?)),
                "-f" | "--file" => cli.files.push(PathBuf::from(value(&mut i)?)),
                "-d" | "--date" => cli.date_opt = Some(value(&mut i)?),
                "-t" | "--title" => cli.title = Some(value(&mut i)?),
                "--headings" => cli.headings = Headings::parse(&value(&mut i)?)?,
                "--no-sync" => cli.no_sync = true,
                other if other.starts_with('-') && other != "-" && cli.cmd.is_some() => {
                    return Err(format!("неизвестная опция `{other}`"));
                }
                _ => {
                    if cli.cmd.is_none() {
                        cli.cmd = Some(a);
                    } else {
                        positional.push(a);
                    }
                }
            }
            i += 1;
        }

        cli.args = positional;
        Ok(cli)
    }

    fn date(&self) -> Res<String> {
        match &self.date_opt {
            Some(d) => {
                notes::check_date(d)?;
                Ok(d.clone())
            }
            None => Ok(notes::today()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Res<Cli> {
        Cli::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn plain_command() {
        let c = cli(&["index"]).unwrap();
        assert_eq!(c.cmd.as_deref(), Some("index"));
        assert!(c.args.is_empty());
        assert_eq!(c.headings, Headings::Shift);
    }

    #[test]
    fn section_is_positional() {
        let c = cli(&["add", "находки"]).unwrap();
        assert_eq!(c.cmd.as_deref(), Some("add"));
        assert_eq!(c.args, vec!["находки"]);
    }

    #[test]
    fn options_with_space_and_equals() {
        let c = cli(&["--root", "/n", "add", "итог", "-d", "2026-09-01"]).unwrap();
        assert_eq!(c.root, Some(PathBuf::from("/n")));
        assert_eq!(c.args, vec!["итог"]);
        assert_eq!(c.date_opt.as_deref(), Some("2026-09-01"));

        let c = cli(&["add", "--root=/n", "--headings=bold", "--no-sync"]).unwrap();
        assert_eq!(c.root, Some(PathBuf::from("/n")));
        assert_eq!(c.headings, Headings::Bold);
        assert!(c.no_sync);
    }

    #[test]
    fn files_accumulate() {
        let c = cli(&["add", "-f", "a.md", "--file", "b.md"]).unwrap();
        assert_eq!(c.files, vec![PathBuf::from("a.md"), PathBuf::from("b.md")]);
    }

    #[test]
    fn errors_are_reported() {
        assert!(cli(&["--root"]).is_err());
        assert!(cli(&["add", "--headings", "wat"]).is_err());
        assert!(cli(&["add", "--nope"]).is_err());
    }

    #[test]
    fn date_defaults_to_today() {
        let c = cli(&["add"]).unwrap();
        assert_eq!(c.date().unwrap(), notes::today());
        let c = cli(&["add", "-d", "2026-9-1"]).unwrap();
        assert!(c.date().is_err());
    }
}
