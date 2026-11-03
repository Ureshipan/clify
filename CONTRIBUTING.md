# Разработка

Нужен актуальный стабильный Rust. Для настольного приложения — графические зависимости X11/Wayland на Linux и установленный `mpv` или `ffplay` для реального воспроизведения.

```sh
cargo run
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

На Linux для отдельной проверки MPRIS установите `dbus-run-session` и `gdbus`:

```sh
dbus-run-session -- env CLIFY_TEST_DBUS=1 cargo test --locked --bin clify mpris::tests::exports_metadata_and_forwards_next_on_session_bus
```

Тесты ядра могут работать без настольных зависимостей:

```sh
cargo test --locked --no-default-features --lib --tests
```

Новые сетевые тесты должны использовать локальный сервер с временными данными. Не добавляйте пароли, API-токены, личные адреса серверов или содержимое чужой медиатеки в исходники и фикстуры. Для запуска настоящей интеграции используйте отдельную тестовую учётную запись и самостоятельно настроенные Navidrome и AudioMuse-AI.

Описание слоёв и ограничений — в [архитектуре](docs/ARCHITECTURE.md), приоритеты — в [плане развития](docs/ROADMAP.md).
