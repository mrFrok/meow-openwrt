# PLAN — meow-openwrt

## Context
Go-форк mihomo-lightweight больше не развиваем (решение пользователя 2026-11-01): Go tiny (23.4 МБ) не помещается в
стоковый флеш (16 МБ, ~8 свободно) и тяжёл для 64–128 МБ ОЗУ. Базой становится meow-rs (Rust, idle RSS 15.7 МБ на BPI-R4),
а исходные пункты Go-плана (TODO.md: урезание лишнего, память, CPU, быстрый путь DIRECT, эксперименты) переносим на Rust.
Форк: `meow-openwrt` (имя предварительное), лицензия **GPLv3**; ядро общедоступное, совместимое с mihomo по конфигу и
REST API, clashwrt-специфики в ядре нет. Цели: mipsel_24kc, mips_24kc, arm_cortex-a7/a9, aarch64.
Источники: `~/Projects/meow-rs` (xerj `mlw-meow`), `~/Projects/clash-rs` (`mlw-clashrs`), `~/Projects/steer` (`mlw-steer`),
deps `mlw-meowdeps-*`, наш Go-форк как эталон поведения (`mlw-fork`). Заметки: `research/meow-rs.md`.

## Жёсткие ограничения (стоковое железо)
- Флеш 16 МБ, свободно ~8 МБ (ath79 бывает 8) → пакет в overlay, без копии в tmpfs. Профиль tiny на mipsel ≤ 7–8 МБ.
- ОЗУ 64 МБ (ath79) / 128 МБ (MT7621) → RSS простоя ≤ ~15 МБ, пик ≤ ~30 МБ.

## Лицензия и upstream
- GPLv3; MIT-уведомления meow-rs сохраняем. Можно брать: mihomo (GPLv3) — портировать поведение напрямую,
  clashrs (Apache-2.0, `tuic` GPLv3), steer (GPLv3), wolfSSL (GPLv3), BoringSSL (Apache-2.0).
- В upstream meow-rs (MIT) — только наши собственные общие правки без GPL-заимствований (паритет, атомики, сборка).

## Этап 0 — форк и сборка (до остального)
1. Форк: полный клон (`git fetch --unshallow` — нужна история ради удалённого rustls-бэкенда), ветка/репо
   `meow-openwrt`, LICENSE GPLv3 + NOTICE (MIT meow-rs), README «неофициальная сборка meow-rs для OpenWrt».
   Публикация на GitHub — только после подтверждения пользователя.
2. Сборка через OpenWrt SDK + feeds rust (1.96; проверить, что meow собирается ниже закреплённых 1.98.1):
   snapshot SDK `ramips/mt7621` в `~/mybuild/sdk-mt7621`, dl/ccache — ссылками на `~/mybuild/openwrt-cache`,
   раннер `bpi-r4-deploy` не трогаем. Пакетный `openwrt/package/<имя>/Makefile` (`rust-package.mk`), boring-sys через
   cmake тулчейна SDK (`OPENSSL_NO_ASM` на MIPS), meow-lwip через `TARGET_CC`.
3. Атомики: 6 файлов с прямым `AtomicU64` → `meow_common::atomic::AtomicU` (meow-listener tun/{udp,policy_route}.rs,
   meow-dns/server.rs, meow-common/outbound_iface.rs, meow-proxy/mux/smux.rs, meow-config/lib.rs).
4. Результат: размер mipsel `minimal`/`full`, `qemu-mipsel meow -v/-t`. Это решает судьбу профилей и этапа 7.
   **Итог 2026-11-01** (SDK snapshot mt7621, rust 1.96, opt z/LTO/panic=abort, stripped):
   `minimal` 10.2 МБ (xz 2.8, apk 4.1), `full` 18.3 МБ (xz 4.8, apk 7.3); оба `-v`/`-t` под qemu-mipsel ок.
   NEEDED: libstdc++ (BoringSSL C++) + libgcc_s → ещё ~1+ МБ на флеше. Сборка: bindgen boring-sys нужен
   `--sysroot` тулчейна; 32-бит musl 1.2 → `--cfg=libc_unstable_musl_v1_2` (time_t 64). Пакет: `openwrt/package/meow`.
   Вывод: для 16 МБ флеша нужен профиль `tiny` (этап 2) и, вероятно, статическая libstdc++ / UPX-подобное.

## Этап 1 — паритет с mihomo (из research; поведение сверять с mihomo/`mlw-fork`)
Сделано 1–4 (коммиты 4530a6a, 3b51b13): сверка API — `bench/api-compare.py`, 19/26 совпадают, остальное —
ключи неподдерживаемых фич (tfo/mptcp/xudp у прокси, geo auto-update, …).
1. Симлинки провайдеров — `meow-config/src/safe_path.rs:52`: лексическая проверка как `C.Path.IsSafePath`.
2. RULE-SET в AND/OR/NOT — `ParserContext` (`meow-rules/src/parser.rs:37`) получает карту rule-set'ов,
   `parse_rule_depth` строит `RuleSetRule` (`rule_set_rule.rs:16`); `meow-config/src/rule_parser.rs:138` — на общий код.
3. Периодический health-check proxy-provider — `meow-config/src/proxy_provider.rs:310` + `meow-tunnel/src/health_check.rs`.
4. REST API: скрипт сверки ответов с mihomo (`PUT /configs?force=true`, `/proxies/{g}`, `/group/{g}/delay`,
   `PUT /providers/rules/{n}`, `/logs?level=`).
5. QUIC-сниффер, `tun.auto-redirect` — позже, по необходимости.

## Этап 2 — урезание лишнего (аналог Go-этапа II: теги → cargo features)
- В meow уже есть тонкие фичи (`docs/specs/cargo-feature-flags.md`: протоколы, транспорты, листенеры, `dns-server`,
  `logbook`, `public-suffix`, `trusttunnel`…) и бандлы `full`/`minimal`. Делаем профили-бандлы:
  - `tiny`: ss, trojan, vless(+vision, reality), hysteria2, transport tls/ws/grpc, listener mixed/tproxy/tun, dns-server;
  - `lightweight`: + anytls, транспорты h2/httpupgrade, relay/load-balance;
  - `full` = upstream.
  Легаси вне tiny/lightweight (как в Go-решении): VMess, Snell, SSR/Hysteria v1 (если есть), kcptun, restls, JLS,
  ShadowTLS, trusttunnel, mux-серверы, inbound-серверы, logbook/SQLite, external-ui, PROCESS-NAME, лишние снифферы.
  Недостающие фичи — добавить (feature-gate) там, где код безусловный.
- Замер вклада каждой фичи: `cargo bloat --crates` / размер при выключении, таблица в BENCHMARKS (как bench/size*).
- Профиль компиляции: уже `opt-level=z`, LTO, `panic=abort`, strip; пробовать `-C target-cpu` под 24kc/cortex-a7,
  `codegen-units=1` (есть). Аналог `-gcflags=-l` — не нужен.
- CI: матрица профилей × архитектур через SDK, SHA-256, `workflow_dispatch` с выбором фич.

## Этап 3 — память (аналог Go-этапа III)
- Аллокатор: проверить фактический (ADR-0008 планировал mimalloc, в `meow-app/src/main.rs` — только dhat); сравнить
  musl malloc / mimalloc на RSS и CPU на BPI-R4 и mipsel.
- Аналог `lw_free_after_load`: `malloc_trim(0)` (musl — `mallctl`/нет; для mimalloc `mi_collect(true)`) после загрузки
  конфига и обновления провайдеров.
- Аналог `lw_auto_memlimit`: лимиты кэшей (DNS, fake-ip, соединения, буферы) от MemTotal, если не заданы в конфиге.
- Пик при загрузке провайдеров (`dhat-heap`), чтение .mrs потоково; окна QUIC (quiche/quinn) в low-memory режиме;
  размер буферов splice/копирования; проверка UDP > 8 КиБ.

## Этап 4 — CPU (аналог Go-этапа IV)
- Замеры CPU %/Мбит на BPI-R4 (стенд из research/meow-rs.md, `bench/prof-run.sh` с обёрткой) — первым делом;
  профилирование `perf` вместо pprof.
- UDP: пакетный приём/отправка (recvmmsg/sendmmsg, GSO/GRO) в TUN/lwIP и в Hysteria2 (наш Go-опыт: −26% CPU/Мбит).
- TUN (lwIP) vs tproxy vs redirect; ChaCha20 vs AES-GCM на mt7621 (через волонтёров); tokio worker-потоки по ядрам
  (`meow-app/src/main.rs:512,540`), `current_thread` для 1 ядра.

## Этап 5 — быстрый путь DIRECT (Go-этап V, не начатый)
- **Сделано (2026-11-03): `dns.nftset`** (docs/nftset.md) — ядро кладёт IP ответов «не-DIRECT» доменов в nft-набор;
  обвязка перехватывает только его (схема clashwrt без 28 МБ списков в dnsmasq). Осталось: перевод clashwrt на него.
- Разгрузка по потокам ниже **отложена до волонтёров с MT7621 (2026-11-03).** На BPI-R4 DIRECT TCP через redir — ~10 Гбит/с при 42 % ядра
  (tunbench MODE=redir), выигрыш HFO там мал. Обучение по IP небезопасно при redir-host (общие IP CDN).
  Безопасный дизайн — решение на один поток (5-tuple), не на IP: (1) TCP при fake-ip — домен известен на SYN,
  DNAT fake→real для этого потока, дальше ядро + flowtable/HFO; (2) UDP — придержать первые датаграммы (SNI QUIC),
  при DIRECT занести 5-tuple в nft-набор с timeout и вернуть пакеты в TUN.
- Модуль `bypass` в ядре: статический анализ правил («IP-определимость»), тесты на таблице AI/RU/CDN-кейсов.
- Поток вердиктов ядро → обвязка: формат, транспорт (unix-сокет или REST/WS), timeout, ct mark — общедоступный интерфейс.
- Обвязка (пакет OpenWrt/clashwrt, не ядро): nft-набор `learned` до перехвата, fw4 flowtable, flush + `conntrack -D --mark`
  при reload/start/stop. Тесты безопасности (BENCHMARKS «Безопасность») до замеров скорости.

## Этап 6 — эксперименты (Go-этап VI)
- sockmap vs splice (после этапа 5); PGO (`cargo pgo`) с профилем с MIPS/ARMv7 у волонтёров.

## Этап 7 — big-endian MIPS (BoringSSL только LE — подтверждено)
rustls + quinn: Reality meow — свой TLS 1.3 на RustCrypto (`meow-transport/src/reality_tls.rs`, из boring только
`boring::aead` → `aes-gcm`/`chacha20poly1305`); обычный TLS — rustls (удалённый бэкенд из истории meow-rs);
Hysteria2 — из clash-rs `clash-lib/src/proxy/hysteria2/` (quinn 0.11, Brutal, Salamander, port hopping);
uTLS/ECH на rustls — открытая задача (образец ручного ClientHello — steer `src/proto/tls/reality.c`).
Старт — после этапа 0.4.
2026-11-03: готовый путь — clash-rs (Apache-2.0): rustls + `ring`, Reality в форке `Watfaq/rustls`
(`clash-lib/src/proxy/transport/reality.rs`, `.with_reality`), hy2 на quinn; `mips-unknown-linux-musl` в их CI
(`-F standard,ring`). Открыто: uTLS-отпечаток/ECH в их форке, размер на MIPS, живость форка rustls.
Первый шаг — собрать clash-rs под mips BE в SDK. Делать после остальных этапов.

## Проверка
- Каждый этап: `cargo test` затронутых крейтов; `meow -t` на боевом конфиге clashwrt (без `cp -rL`, без предупреждений).
- Сборки: SDK-пакеты 4 архитектур, `qemu-mipsel`, BPI-R4 (SSH), волонтёры — self-run скрипт (без SSH).
- Замеры ≥3 прогона, таблицы в BENCHMARKS форка; после правок — `xerj autoindex <форк> --prefix mlw-meow --fresh`.

## Порядок
rustls-бэкенд удалён в upstream коммитом 5f6902e (2026-09-06) — восстанавливать оттуда (этап 7).

0 → 1 ∥ 2 → 4 (замеры) → 3 → 5 → 6; 7 — по итогам 0.4.
