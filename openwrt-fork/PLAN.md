# PLAN — meow-openwrt

## Context
Go-форк mihomo-lightweight больше не развиваем (решение пользователя 2026-11-01): Go tiny (23.4 МБ) не помещается в
стоковый флеш (16 МБ, ~8 свободно) и тяжёл для 64–128 МБ ОЗУ. Базой становится meow-rs (Rust, idle RSS 15.7 МБ на BPI-R4),
а исходные пункты Go-плана (TODO.md: урезание лишнего, память, CPU, быстрый путь DIRECT, эксперименты) переносим на Rust.
Форк: `meow-openwrt` (имя предварительное), лицензия **GPLv3**; ядро общедоступное, совместимое с mihomo по конфигу и
REST API, никакой специфики конкретных обвязок в ядре нет: ядро работает у всех. Цели: mipsel_24kc, mips_24kc, arm_cortex-a7/a9, aarch64.
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

## Этап 3 — память (аналог Go-этапа III) — ✅ закрыт 2026-11-06
- ✅ **Аллокатор (2026-11-06)**: musl mallocng остаётся. meowmem, tiny aarch64, живой конфиг, 2 прогона, RssAnon КБ:
  musl простой 5.2–5.5 / пик старта ~7 / 50 потоков 9.0; mimalloc 34–50 / 58–64 / 28–48, с
  `MIMALLOC_PURGE_DELAY=0 ARENA_EAGER_COMMIT=0` 9.7 / 14 / 13. Feature `mimalloc` остаётся опцией, не в профилях.
- ✅ **Аналог `lw_free_after_load` не нужен**: mallocng сам отдаёт память (пик старта ~7–8.5 МБ → 5.2 МБ в простое).
- ✅ **Память на соединение (dhat, 200 потоков)**: async-обёртки `ConnNotes::scope`/`run_until_closed` удваивали фрейм
  задачи (аргумент-фьючер в префиксе генератора + awaitee). Исправлено: задача mixed 24.8 → 13.0 КБ, общий
  `route_inbound_tcp` 22.1 → 11.2 КБ (daeed6d, b14c9d3; годится в upstream). Остаток на поток: 8 КБ стековых буферов,
  +32 КБ bulk-буфер у качающих (паритет с mihomo).
- ✅ **`max-connections` по умолчанию от MemTotal**: MemTotal/8/32 КиБ, 256..16384 (64 МБ → 256, 256 МБ → 1024).
  При лимите accept встаёт в очередь — 256 на домашнем шлюзе мало.
- ✅ **Пик загрузки провайдеров**: text/yaml идут в сборщики потоком (d55c820), метки trie — в одной арене, которую
  забирает запечатанное дерево (4cac7e7). 80k доменов text: пик 15.6 → 10.2 МБ (dhat), аллокаций на сборку ~100×
  меньше (DomainRuleSet 100k: 11322 → 96). Удержано без изменений (~15 Б/домен).
- ✅ UDP > 8 КиБ: все UDP-пути (TUN, TPROXY, SOCKS5, SS) читают в 64 КиБ.
- Остаток (не блокирует): bulk-буфер 32 КиБ можно уменьшать на ≤128 МБ; пик загрузки ещё содержит весь файл
  провайдера (1.6 МБ на 80k) и временные таблицы сборщика (~7 МБ на 100k) — потоковое чтение файла и CSR без
  HashMap рёбер, если понадобится.
- Upstream (MIT, в очередь): удвоение фрейма (daeed6d, b14c9d3), потоковые провайдеры (d55c820), арена trie (4cac7e7).
- Аналог `lw_auto_memlimit`: лимиты кэшей (DNS, fake-ip, соединения, буферы) от MemTotal, если не заданы в конфиге.
- Пик при загрузке провайдеров (`dhat-heap`), чтение .mrs потоково; окна QUIC (quiche/quinn) в low-memory режиме;
  размер буферов splice/копирования; проверка UDP > 8 КиБ.
- **2026-11-06 замеры (`meow-dns/tests/fakeip_footprint.rs`)**: полный fake-ip /16 в памяти = 22.8 МиБ (356 Б/запись),
  DNS-кэш 4096 = 1.9 МиБ. Сделано: in-memory fake-ip store ограничен MemTotal/16 (~400 Б/запись, мин. 4096):
  64 МБ → 10.5k, 128 МБ → 21k, ≥512 МБ → весь /16; пул по-прежнему обходит весь диапазон. nft-сет
  `dns.nftset`: ~105 Б ядра на IPv4-элемент.

## Этап 4 — CPU (аналог Go-этапа IV)
- Замеры CPU %/Мбит на BPI-R4 (стенд из research/meow-rs.md, `bench/prof-run.sh` с обёрткой) — первым делом;
  профилирование `perf` вместо pprof.
- UDP: пакетный приём/отправка (recvmmsg/sendmmsg, GSO/GRO) в TUN/lwIP и в Hysteria2 (наш Go-опыт: −26% CPU/Мбит).
- TUN (lwIP) vs tproxy vs redirect; ChaCha20 vs AES-GCM на mt7621 (через волонтёров); tokio worker-потоки по ядрам
  (`meow-app/src/main.rs:512,540`), `current_thread` для 1 ядра.

## Этап 5 — быстрый путь DIRECT (Go-этап V)
- **Сделано (2026-11-03): `dns.nftset`** (docs/nftset.md) — ядро кладёт IP ответов «не-DIRECT» доменов в nft-набор;
  файрвол перехватывает только его (без списков доменов в dnsmasq: 80k доменов там ~28 МБ, в ядре ~1.2 МБ).
- **Сделано (2026-11-06): перехват без обвязки — `tun.auto-redirect`** (0b738f7, 3665619; docs/tun.md). Ключ
  mihomo: таблица `inet meow` как у sing-tun (redirect TCP host+LAN во внутренний SO_ORIGINAL_DST-листенер,
  UDP/ICMP fwmark 0x2023 → policy rules 9200–9205 → TUN, DNS dnat → TUN-пир), свои подсети/lo не трогает,
  bypass-mark 0x2024 сам, include для fw4, уборка при выходе. Сужение: `route-address(-set)` (ipcidr rule-sets),
  `route-exclude-address(-set)`; meow-ключ **`route-address-dns: true`** — сет `route4_dns` (timeout) наполняет
  хук dns.nftset, т.е. в ядро идёт только «не-DIRECT», остальное мимо: обвязке достаточно ключей в конфиге
  (заменяет `dns.nftset.create`). `bench/autoredirect-test.sh` (3 netns на BPI-R4): всё PASS.
  Сеты следуют за refresh rule-set без рестарта (фаза 3); IPv6 при `inet6-address` (фаза 4); отдельный opt-in `block-encrypted-dns` — reject 853 и DoH к известным резолверам (+`block-encrypted-dns-address`), фаза 5. Осталось: повторная
  установка, если таблицу снесли извне (`fw4 reload` её не трогает — своя таблица).
- Разгрузка по потокам ниже **отложена до волонтёров с MT7621 (2026-11-03).** На BPI-R4 DIRECT TCP через redir — ~10 Гбит/с при 42 % ядра
  (tunbench MODE=redir), выигрыш HFO там мал. Обучение по IP небезопасно при redir-host (общие IP CDN).
  Безопасный дизайн — решение на один поток (5-tuple), не на IP: (1) TCP при fake-ip — домен известен на SYN,
  DNAT fake→real для этого потока, дальше ядро + flowtable/HFO; (2) UDP — придержать первые датаграммы (SNI QUIC),
  при DIRECT занести 5-tuple в nft-набор с timeout и вернуть пакеты в TUN.
- Модуль `bypass` в ядре: статический анализ правил («IP-определимость»), тесты на таблице AI/RU/CDN-кейсов.
- Поток вердиктов ядро → обвязка: формат, транспорт (unix-сокет или REST/WS), timeout, ct mark — общедоступный интерфейс.
- Обвязка (не ядро): nft-набор `learned` до перехвата, fw4 flowtable, flush + `conntrack -D --mark`
  при reload/start/stop. Тесты безопасности (BENCHMARKS «Безопасность») до замеров скорости.

## Этап 6 — эксперименты (Go-этап VI)
- sockmap vs splice (после этапа 5); PGO (`cargo pgo`) с профилем с MIPS/ARMv7 у волонтёров.
- **PGO на BPI-R4 (2026-11-03, `bench/pgo-gen.sh`, `MEOW_PGO_PROFILE=`)**: профиль из tunbench + старт на живом
  конфиге, замер на тех же сценариях (оптимистично). %CPU/Мбит: hy2 UDP ↓ 1.31→0.98 (−25%, +11% скорости),
  hy2 TCP ↓ 0.49→0.46, ↑ 0.43→0.39; hy2 UDP ↑ хуже на 6%; DIRECT TCP без изменений. Размер tiny +327 КБ
  (jffs2 4.03→4.29 МБ). Нет Reality в профиле; на MIPS не проверено. По умолчанию выключено.
- **PGO + Reality (2026-11-05)**: tunbench получил `vrtcp-ul/dl` (VLESS+Vision+REALITY, сервер mihomo в netns,
  dest = его TLS-контроллер, ключи из `bench/reality-*.yaml`). Профиль: все сценарии tunbench, Reality ×2.
  `pgo-gen.sh` сам пишет cmake-toolchain (без прошлой SDK-сборки), `PGO_USE=<profdata>|none` — оптимизированная
  и базовая сборки тем же rustup 1.96 (llvm-profdata из его llvm-tools). base vs pgo, 2 прогона, %CPU/Мбит:
  Reality ↑ 0.08→0.07 (+2% скорости), ↓ 0.075→0.065; hy2 TCP ↑ 0.42→0.30 (+38% скорости), ↓ 0.42→0.35 (+26%);
  hy2 UDP ↑ 0.93→0.70, ↓ 0.75→0.70; DIRECT TCP ↑ +15% скорости. Размер 8.42→8.94 МБ (+524 КБ).
  Оптимистично (обучение = замер). Профиль ~/.cache/meow-pgo/meow.profdata (17 МБ, не в git).
- **hy2 UDP без копий (2026-11-06, 6328cba, 08f9376)**: `read_packet_owned` (ответ hy2 → TUN/TPROXY без
  копии в scratch и `to_vec`), отправка кодируется в переиспользуемый буфер, адреса SmolStr. tunbench, оба PGO,
  2 прогона, %CPU/Мбит: hy2 ↑ 300M 0.79→0.62 (шумно), ↓ 100M 0.80→0.74, ↑ 100M без изменений. ↓ упирается
  в ~90 Мбит/с при 70% потерь при 300M — узкое место не в копиях.
  Разобрано: это стенд. mihomo-клиентом те же 89 Мбит/с; серверный mihomo и iperf3 -s делят ядро 3. С
  `SRV_CPU=2,3 CLI_CPU=2` ↓ 160 Мбит/с у обоих (meow 0.65 %/Мбит, mihomo 0.73). Brutal (`down:`) не помогает.
  Клиент hy2 ↓ узким местом не является; честный замер ↓ выше ~150 Мбит/с на 4 ядрах BPI-R4 не получить.
- sockmap/eBPF: не делаем — DIRECT TCP уже splice ~10 Гбит/с, с dns.nftset мимо ядра; шифрование в BPF невозможно.

## Этап 7 — big-endian MIPS (BoringSSL только LE — подтверждено)
Сделано 2026-11-06 (бэкенд выбирают build.rs: `cfg(boring)` в meow-transport, `cfg(hy2_quiche)` в meow-proxy;
LE ничего не теряет; `MEOW_TLS_BACKEND=rustls` включает BE-путь на хосте для тестов):
- 7.1 крипто-фасад `meow_transport::crypto` — BoringSSL на LE, RustCrypto на BE (sniffer QUIC, AEAD
  VMess/Snell/VLESS-enc, REALITY, ShadowTLS).
- 7.2 TLS на rustls + ring (`tls/rustls_backend.rs`): SNI/ALPN/версии/pin/mTLS/curves/distrust. Нет на BE:
  uTLS-отпечатков (warn), ECH (ошибка конфига), Vision-passthrough, restls/jls (compile_error).
- 7.3 REALITY + Vision на BE — проверено под qemu-mips против mihomo-сервера.
- 7.4 hysteria2 на quinn (`hysteria2/driver_quinn.rs`, тот же `Cmd`-протокол, BBR, Salamander/Gecko,
  port hopping); общий static-only QPACK (`qpack.rs`, Huffman через `octets`). TCP+UDP проверены под qemu.
- 7.5 сборка: `bench/mips-be-build.sh` (ath79 SDK, build-std). tiny = 11 МБ статический stripped
  (xz 2,9 / gzip 4,6 МБ; на UBIFS ≈ 5,5 МБ zlib / 5,7 zstd / 7,5 lzo — поблочная оценка, zlib сверена с
  mkfs.ubifs). Цель «< 6 МБ несжатым» снята: ядро само ~6 МБ, ориентир — паритет с LE (tiny aarch64 8,5).
Открыто: проверка на живом BE-железе (волонтёры), скорость RustCrypto/ring на 24Kc, Brutal на quinn.

## Проверка
- Каждый этап: `cargo test` затронутых крейтов; `meow -t` на боевом конфиге роутера (без `cp -rL`, без предупреждений).
- Сборки: SDK-пакеты 4 архитектур, `qemu-mipsel`, BPI-R4 (SSH), волонтёры — self-run скрипт (без SSH).
- Замеры ≥3 прогона, таблицы в BENCHMARKS форка; после правок — `xerj autoindex <форк> --prefix mlw-meow --fresh`.

## Порядок
rustls-бэкенд удалён в upstream коммитом 5f6902e (2026-09-06) — восстанавливать оттуда (этап 7).

0 → 1 ∥ 2 → 4 (замеры) → 3 → 5 → 6; 7 — по итогам 0.4.
