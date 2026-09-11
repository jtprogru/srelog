//! Команды целиком: что srelog пишет в stdout и stderr и с каким кодом выходит.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const TABLE: &str = "\
| Направление | Что завести | Основание |
|-------------|-------------|-----------|
| tooling | Тул quota_check | Бридж не видит квоты |
| storage | Обновить драйвер | Тихая порча данных |
| infra | Дашборд квот | Смотрели руками |
";

/// Журнал с одной записью за 2026-09-03 во временной папке; имя уникально по тесту.
fn journal(tag: &str, table: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("srelog-cli-{}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let year = root.join("oncall").join("2026");
    fs::create_dir_all(&year).unwrap();
    fs::write(
        year.join("2026-09-03.md"),
        format!("---\ndate: 2026-09-03\n---\n\n## Кандидаты в бэклог\n\n{table}"),
    )
    .unwrap();
    root
}

/// Запускает srelog на журнале. Окружение пользователя не подмешивается.
fn srelog(root: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_srelog"))
        .arg("--root")
        .arg(root)
        .args(args)
        .env_remove("SRELOG_ROOT")
        .env("ONCALL_ENGINEER", "test")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("srelog запускается");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn sync_without_list_prints_only_paths() {
    let root = journal("sync-nolist", TABLE);
    let out = srelog(&root, &["sync"], "");

    assert!(out.status.success(), "код возврата: {:?}", out.status);
    assert_eq!(text(&out.stderr), "");
    let oncall = root.join("oncall");
    assert_eq!(
        text(&out.stdout),
        format!(
            "{}\n{}\n",
            oncall.join("INDEX.md").display(),
            oncall.join("BACKLOG.md").display()
        )
    );
    // без списка порядок прежний: по строке целиком
    let backlog = fs::read_to_string(oncall.join("BACKLOG.md")).unwrap();
    assert!(
        backlog.ends_with(
            "\
| infra | Дашборд квот | Смотрели руками | [2026-09-03](2026/2026-09-03.md) |
| storage | Обновить драйвер | Тихая порча данных | [2026-09-03](2026/2026-09-03.md) |
| tooling | Тул quota_check | Бридж не видит квоты | [2026-09-03](2026/2026-09-03.md) |
"
        ),
        "{backlog}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn sync_warns_about_unknown_direction_and_keeps_going() {
    let root = journal("sync-list", TABLE);
    fs::write(
        root.join("srelog.toml"),
        "directions = [\"tooling\", \"infra\"]\n",
    )
    .unwrap();
    let out = srelog(&root, &["sync"], "");

    assert!(out.status.success(), "код возврата: {:?}", out.status);
    let entry = root.join("oncall/2026/2026-09-03.md");
    assert_eq!(
        text(&out.stderr),
        format!(
            "srelog: {}:10: направления `storage` нет в srelog.toml\n",
            entry.display()
        )
    );
    let backlog = fs::read_to_string(root.join("oncall/BACKLOG.md")).unwrap();
    assert!(
        backlog.ends_with(
            "\
| tooling | Тул quota_check | Бридж не видит квоты | [2026-09-03](2026/2026-09-03.md) |
| infra | Дашборд квот | Смотрели руками | [2026-09-03](2026/2026-09-03.md) |
| storage | Обновить драйвер | Тихая порча данных | [2026-09-03](2026/2026-09-03.md) |
"
        ),
        "{backlog}"
    );

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn add_writes_row_with_unknown_direction_and_warns() {
    let root = journal("add-list", TABLE);
    fs::write(
        root.join("srelog.toml"),
        "directions = [\"tooling\", \"infra\", \"storage\"]\n",
    )
    .unwrap();
    let out = srelog(
        &root,
        &["add", "бэклог", "-d", "2026-09-03"],
        "| infra-hw | Заменить диск | SMART ругается |\n",
    );

    assert!(out.status.success(), "код возврата: {:?}", out.status);
    let entry = root.join("oncall/2026/2026-09-03.md");
    let written = fs::read_to_string(&entry).unwrap();
    let line = 1 + written
        .lines()
        .position(|l| l.contains("infra-hw"))
        .expect("строка не записалась");
    assert_eq!(
        text(&out.stderr),
        format!(
            "добавлено в «Кандидаты в бэклог»\n\
             srelog: {}:{line}: направления `infra-hw` нет в srelog.toml\n",
            entry.display()
        )
    );
    assert!(fs::read_to_string(root.join("oncall/BACKLOG.md"))
        .unwrap()
        .contains("| infra-hw | Заменить диск | SMART ругается | [2026-09-03]"));

    let _ = fs::remove_dir_all(&root);
}
