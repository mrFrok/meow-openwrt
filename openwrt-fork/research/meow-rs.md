# meow-rs — mihomo на Rust (разведка 2026-11-01)

Цель следующей сессии: оценить работу по доведению meow-rs до функциональности mihomo,
нужной clashwrt, особенно для слабых роутеров (64–128 МБ RAM, MIPS).

## Что это

- https://github.com/meow-rs/meow-rs — «A high-performance Rust implementation of the mihomo
  (Clash Meta) proxy kernel». MIT. Создан 2026-02-21, коммиты ежедневно (последний 2026-11-01).
  614 ★, 83 форка, 224 issue (открыты 2: #869, #870).
- Авторы: madeye 613 коммитов (судя по нику — Max Lv, автор shadowsocks-libev/-android), aimkiray 140,
  gengjiawen 108, «claude» 29 (разработка с ИИ, в репо есть CLAUDE.md/AGENTS.md).
- Релиз v0.28.0 (2026-10-30), плюс ночные alpha. Тулчейн закреплён: Rust 1.98.1 (rust-toolchain.toml),
  MSRV 1.89. Release-профиль: `opt-level="z"`, `lto=true`, `codegen-units=1`, `panic="abort"`, `strip`.
- Крейты: meow-anytls, meow-api, meow-app, meow-bench, meow-cli, meow-common, meow-config, meow-dns,
  meow-listener, **meow-lwip** (TUN-стек на lwIP), meow-proxy, meow-rules, meow-transport, meow-trie, meow-tunnel.
- Каталог `openwrt/`: build-ipk.sh, build-apk.sh, docker, **luci-app-meow**, пакет meow.
- Криптография — только BoringSSL (вендорен): TLS, uTLS-отпечатки, ECH, DoT/DoH, QUIC для Hysteria2.

## Возможности (README + исходники)

- Исходящие: Shadowsocks (AEAD, stream, SS2022?), Trojan, VMess (AEAD; TCP/WS), VLESS (plain, XTLS-Vision splice;
  транспорты TLS/WS/gRPC/H2/HTTPUpgrade), **Reality** (`crates/meow-transport/src/reality_hello.rs`,
  `reality_tls.rs`; в README не упомянута), Hysteria2 (Salamander, port hopping, pinning), AnyTLS
  (feature `anytls`, есть в релизах), HTTP CONNECT, SOCKS5, mux.cool.
- Группы: selector, url-test, fallback, load-balance, relay, `dialer-proxy` (с собственными улучшениями цепочек).
- Входящие: mixed/HTTP/SOCKS5, **TProxy** (`tproxy-port`, `routing-mark`, `tproxy-sni`), TUN (Linux, Wintun).
- Правила: стандартные + AND/OR/NOT; rule-providers (читает `.mrs`), proxy-providers, подписки через API.
- DNS: fake-ip, snooping, DoT/DoH; сниффер (без QUIC).
- REST API в стиле mihomo + свои `/api/...` (subscriptions, proxy-groups, config/save), веб-UI `/ui`,
  «logbook» (журнал событий, переживает перезапуск), `meow-cli`.
- CLI совместим: `-d <dir>`, `-f <config>`, `-t`, `--ext-ctl`, `-v` → «Meow Meta 0.28.0».

## Цифры

README (их замер, loopback, против mihomo): бинарник 54.0 → 7.8 МБ, RSS в простое 31.3 → 9.7 МБ,
RSS пик 42.4 → 12.2 МБ, TCP 64 МБ×1 14.4 → 14.9 Гбит/с, 1 МБ×10 17.2 → 16.8.

Наше (BPI-R4, aarch64, v0.28.0 `aarch64-unknown-linux-musl`, конфиг clashwrt):
- бинарник 12.6 МБ (static, stripped; наш tiny arm64 23.5 МБ, tiny mipsle 23.4 МБ);
- простой: **RSS 15.7 МБ, anon 6.7 МБ, CPU 0.2%** (наша отладочная сборка lightweight с pprof: 63.1 / 36.5 МБ / 1.8%;
  боевой mihomo v1.19.33: RSS ~83 МБ, anon 45 МБ);
- CPU на Мбит/с **не измерен**: TCP через Reality не прошёл на обеих сборках (стенд: «control socket has closed
  unexpectedly» у нашей, «Connection refused» у meow — не разобрано), прогоны UDP через Hysteria2 сорвались
  (ноутбук-клиент ушёл из сети). Это первое, что нужно измерить.
- Сборок под MIPS нет (релизы: aarch64 ×12, armv7, x86_64, i686, riscv64).

## Несовместимости с конфигом clashwrt (`meow -t` на /etc/mihomo)

1. **Символьные ссылки**: clashwrt делает `/etc/mihomo/{providers,ru-bundle,re-filter,geo}` → `/tmp/clashwrt-prov/...`;
   meow отказывается: «path … escapes the provider directory via a symlink» — фатально для rule-provider
   (и proxy-provider не создаётся). mihomo это принимает. Обход в стенде: копия `cp -rL` (обёртка `/tmp/prof/meow-wrap`).
2. **RULE-SET внутри логических правил**: `AND,((NETWORK,udp),(RULE-SET,ru_bundle)),ProxyUDP` → «unknown rule type:
   RULE-SET», правило пропускается. На этом построен UDP-маршрут clashwrt (ProxyUDP).
3. Игнорируются: `tun.stack`, `tun.auto-detect-interface`, `tun.auto-redirect`, `sniffer.force-dns-mapping`
   (по смыслу всегда включено), `sniffer.sniff.QUIC` (не реализован).
4. proxy-provider `health-check` хранится, но периодически не запускается (только ручной/групповой).
5. Совместимость REST API с вызовами clashwrt не проверена (`PUT /configs?force=true {"path":…}`, `PUT /proxies/<g>`
   ответили 204 в стенде).

## Риски / замечания по зрелости

- Reality: #708 (закрыт) — `client-fingerprint` молча игнорировался, ClientHello был фиксированной минимальной формы
  (1 шифр, 9 расширений, без GREASE) — легко детектируется; #430 (закрыт) — неограниченный рост transcript до
  аутентификации Reality (удалённый OOM). #408 — числовой short-id без кавычек ронял узел.
- #570 (закрыт): relay-группы не завершались на vless/trojan/anytls/ss. #424: mux.cool + vision.
- #869/#870 открыты: anytls молча игнорирует ech-opts; хвосты по снифферу.
- Rust под mipsel — tier 3 (нет готовой std): nightly + `-Zbuild-std`, кросс-сборка BoringSSL (cmake) под mipsel softfloat.
  meow-lwip — C-код lwIP, тоже кросс-сборка.

## Что предлагалось

- Не форкать, а отправлять исправления в meow-rs (п.1, 2, QUIC-сниффер, mipsel-сборка полезны всем),
  в clashwrt — выбор ядра (mihomo / meow).
- Порядок оценки: (1) CPU на Мбит/с TCP-Reality и UDP-Hysteria2 против нашего lightweight на том же стенде;
  (2) прогнать боевой конфиг clashwrt и список различий; (3) пробная сборка mipsel (tier 3 + BoringSSL) и RSS на ней;
  (4) объём доработок по п.1–5.

## Стенд (как воспроизвести)

- На роутере: `/tmp/prof/meow` (бинарник), `/tmp/prof/meowrun/mihomo` (копия, чтобы `pidof mihomo` в bench работал),
  `/tmp/prof/meow-wrap` — обёртка: `-v` → версия; иначе `cp -rL /etc/mihomo /tmp/meowd` и
  `exec /tmp/prof/meowrun/mihomo -d /tmp/meowd -f /tmp/meowd/config.yaml`. tmpfs — пропадёт после перезагрузки.
- Скачать: `gh release download v0.28.0 -R meow-rs/meow-rs -p 'meow-v0.28.0-aarch64-unknown-linux-musl.tar.gz*'`.
- Запуск: `PROXY_INJECT=<pc-client.yaml> TCP_TARGET="LW Reality" UDP_TARGET="LW Hysteria2" UDP_LEN=1200
  VPS=203.0.113.3 IPERF_SSH=mrfrok@192.168.1.108 UDP_BW=200M bench/prof-run.sh mw-meow udp-proxy-dl /tmp/prof/meow-wrap`
  (сервер Hysteria2/Reality и iperf3 на ПК 192.168.1.231 с 203.0.113.3 на lo; клиент iperf3 на ноутбуке).
  pprof у meow нет — `*.cpu.pprof` пустой, это нормально.
- Во время прогонов meow трафик ПК через роутер может пропадать (gh api давал TLS timeout).
- Наши числа для сравнения (стенд по кабелю, сервер на ПК): Hysteria2 отдача 200M — 123 Мбит/с, 1.17 %/Мбит
  (BENCHMARKS.md); приём (стенд с ноутбуком-сервером) — 240 Мбит/с, 0.65–0.67 %/Мбит.

## Разведка по исходникам (2026-11-01, клон ~/Projects/meow-rs @5c59374, индекс xerj `mlw-meow`)

Принцип: исправления — общие, с паритетом mihomo, в upstream; ничего clashwrt-специфичного в ядре.

1. **Symlink** (`crates/meow-config/src/safe_path.rs:52`): `resolve_contained` после лексической проверки
   канонизирует оба пути и сравнивает. Уже есть escape hatch как в mihomo: `SKIP_SAFE_PATH_CHECK=1` → warn и пропуск
   (обход без правки кода). Разница с mihomo: `C.Path.IsSafePath` у mihomo лексический, симлинки не разворачивает —
   поэтому mihomo конфиг принимает. Upstream-предложение: паритет (лексическая проверка) либо опция; ~20 строк.
2. **RULE-SET в AND/OR/NOT**: RULE-SET разбирается только в meow-config (`rule_parser.rs:138 try_parse_rule_set`,
   нужна карта провайдеров), а логические правила рекурсивно уходят в `meow_rules::parse_rule_depth`
   (`crates/meow-rules/src/parser.rs:292,382`), где RULE-SET неизвестен (`parser.rs:239`). `RuleSetRule` уже
   живёт в meow-rules (`rule_set_rule.rs:16`). Правка: положить карту `name → Arc<dyn RuleSet>` в `ParserContext`
   и обработать RULE-SET в `parse_rule_depth` (+ hints из #276). ~50–100 строк + тесты. Полезно всем (mihomo это умеет).
3. tun.stack/auto-redirect/auto-detect-interface/QUIC-сниффер — не смотрел глубже; QUIC-сниффер — отдельная задача
   (расшифровка Initial, нужен AES-GCM/HKDF из BoringSSL — есть).
4. **proxy-provider health-check** (`proxy_provider.rs:310`): честно предупреждает, что по таймеру не запускается.
   Инфраструктура есть (`meow-tunnel/src/health_check.rs`) — нужно повесить периодический запуск. ~50 строк.
5. **REST API, используемый clashwrt** — все маршруты есть (`crates/meow-api/src/routes.rs:195-283`):
   `PUT /configs`, `GET/PUT/DELETE /proxies/{name}`, `/proxies/{name}/delay`, `/group/{name}/delay`,
   `PUT /providers/rules/{name}`, `/providers/proxies/{name}[/healthcheck]`, `/logs?level=` (WebSocket и
   HTTP-поток JSON для wget). Поведение (коды, формат) — проверить на стенде.

### Главный блокер для слабых роутеров: TLS = только BoringSSL

- `.github/workflows/build.yml:56`: «Targets that cannot link boring-sys (MIPS, 32-bit musl, windows-gnu) are not
  shipped and — since the rustls backend was removed — cannot be built with TLS at all». armv7 в релизах — только
  `gnueabihf` (glibc) → **на OpenWrt не годится**; т.е. из OpenWrt-целей есть лишь aarch64/x86_64/riscv64 musl.
- Hysteria2 = quiche на той же BoringSSL. Фичи `full`/`minimal` обе требуют boring-sys (`crates/meow-app/Cargo.toml`).
- mipsel: ADR-0007 — «soft-gated», нет инфраструктуры; `docs/benchmarks/binary-size.md:120` — только nightly
  `-Zbuild-std` (компиляция std стартует, до конца не доведено).
- Варианты: (а) довести кросс-сборку boring-sys под mipsel/armv7 musl (BoringSSL с `OPENSSL_NO_ASM`, zig/cmake;
  причина отказа на 32-bit musl неизвестна — проверить первым делом); (б) вернуть rustls+ring как второй бэкенд
  (ring собирается под mips/armv7) — большая работа: uTLS/ECH/Reality/quiche завязаны на BoringSSL.
- Локально: есть rustup(stable)/cargo/cmake; нет nightly, zig, cargo-zigbuild.

### Оценка (предварительно)
п.1, 2, 4 — дни работы, хорошо ложатся в upstream. Главный объём и риск — 32-bit musl/MIPS (вариант а: неизвестно,
от дней до недель; вариант б: недели). CPU на Мбит/с всё ещё не измерен.

## Фаза 0 плана (2026-11-01; план: ~/.claude/plans/streamed-soaring-popcorn.md)

- **BoringSSL и big-endian — подтверждено, не работает**: `boring-sys-5.2.0/deps/boringssl/include/openssl/target.h`
  знает только `__MIPSEL__` (MIPS LE), для прочих — `#error "Unknown target CPU"` с пояснением «only … little-endian».
  → mips_24kc (ath79, BE) только через фазу 6 (rustls+ring), mipsel_24kc и ARM32 (`__ARMEL__`) — BoringSSL поддерживает.
- **64-битные атомики**: upstream уже имеет шим `crates/meow-common/src/atomic.rs` (`AtomicU`/`AtomicI` → 32 бит без
  `target_has_atomic="64"`, 62 использования). Прямые `AtomicU64` остались в ~10 файлах (meow-api/routes.rs,
  meow-common/{outbound_iface,logbook/store}.rs, meow-dns/server.rs, meow-listener/tun/{udp,policy_route}.rs,
  meow-config/lib.rs, meow-proxy/mux/smux.rs, trusttunnel, bench) — часть может быть под cfg; точно покажет сборка.
  В зависимостях: quiche/hickory/h2/hyper/dashmap — без AtomicU64; tokio и crossbeam-utils — с cfg-защитой.
- Пробная сборка (п.0.3) не делалась: нужен OpenWrt SDK + сборка rust/host из feeds (долго, часы).
- Уточнение через xerj (`mlw-meow`, AtomicU64 без `target_has_atomic` в файле): в рабочем коде остаются
  meow-listener/tun/{udp,policy_route}.rs, meow-dns/server.rs, meow-common/outbound_iface.rs, meow-proxy/mux/smux.rs,
  meow-config/lib.rs (+ logbook/store.rs и trusttunnel — вне профиля router). Правка: перевести на
  `meow_common::atomic::AtomicU`, один небольшой PR.
- Исходники зависимостей в xerj: `mlw-meowdeps-boring-sys` (boring-sys 5.2.0 с BoringSSL), `mlw-meowdeps-quiche` (0.30.0).

## Ограничения стокового железа (поправка 2026-11-01)
Данные волонтёров — кастомные ПЗУ/ОЗУ; сток заметно меньше: флеш 16 МБ (~8 МБ свободно, ath79 бывает 8 МБ),
ОЗУ 64 МБ (ath79) / 128 МБ (MT7621). Следствия:
- Go tiny (23.4 МБ) во флеш не помещается, а из tmpfs съедает ОЗУ → fallback «mips BE → Go tiny» для стока неверен.
- meow: ADR-0007 задаёт для mipsel minimal ≤ 7 MiB (ss+trojan+dns+mixed); профиль router (+vless/reality/hy2)
  нужно уложить в ~7–8 МБ — проверить сжатие overlay (jffs2/ubifs) на реальной установке.
- mips BE: OpenSSL-бэкенд без Reality/Hysteria2/uTLS (quiche 0.30 только BoringSSL) бесполезен; mihomo там работает,
  т.к. весь TLS на Go (metacubex/tls, utls v1.8.8, quic-go/sing-quic). Для meow — только rustls+ring + свой
  Reality/uTLS (фаза 6), решение после замеров mipsel.

## steer (github.com/splify2/steer) — справочно (клон ~/Projects/steer, xerj `mlw-steer`)
C, GPLv3, v2.0.5. Не ядро mihomo: своя спека YAML → nftables/fake-IP; протоколы — модули с TUN на выход.
Собирается под mips_24kc (BE), mipsel_24kc, arm_cortex-a7/a9, aarch64, x86_64; ipk mips_24kc: core 1.1 МБ,
vless 49 КБ, hysteria2 83 КБ. TLS — свой билд wolfSSL с QUIC (`libsteer-wolfssl.so`), QUIC — ngtcp2 с патчем Brutal;
Reality — ClientHello собран вручную (`src/proto/tls/reality.c`), сверен с uTLS Chrome 133 (`tests/hellostruct.py`).
Вывод: связка wolfSSL+ngtcp2+ручной ClientHello решает BE MIPS с Reality и Hysteria2 в ~1.3 МБ. Код GPLv3 —
в MIT-ядро (meow) копировать нельзя, только как образец подхода.

## clash-rs (Watfaq, клон ~/Projects/clash-rs @36eec77, xerj `mlw-clashrs`) — что можно взять
- Лицензия: Apache-2.0; GPLv3 — только при фиче `tuic` (tuic-core) (NOTICE.md). clash-dns — MIT, netstack — MIT/Apache.
  Apache-код в MIT-проект брать можно (сохранить LICENSE/NOTICE).
- TLS: rustls 0.23 с патчем `Watfaq/rustls` (ветка watfaq/0.23.45) ради `with_reality(...)`; провайдер ring или aws-lc-rs.
  uTLS-отпечатков (`client-fingerprint`) и ECH нет → их Reality ClientHello rustls-образный (детектируемый).
- Hysteria2: quinn 0.11, `clash-lib/src/proxy/hysteria2/` ~2300 строк: codec, Brutal (`congestion.rs` на
  `quinn_proto::congestion::Controller`), Salamander, port hopping (`udp_hop.rs`). — лучший кандидат для BE-бэкенда meow.
- Reality у meow лучше (свой TLS 1.3 + отпечатки), брать у clash-rs не нужно.

## UDP через TUN и Hysteria2 на BPI-R4 (2026-11-02, `bench/tunbench.sh`)

Стенд целиком в двух netns на роутере (хост и боевой mihomo не затронуты): iperf3 в `mwc` → TUN ядра
(`routing-mark` 0x99 уводит исходящие сокеты ядра в veth) → `mwb` (iperf3 -s, сервер Hysteria2 — mihomo
listener на CPU 3). 1200-байтовые датаграммы, ядро на CPU 0,1. Потолок hy2 — сервер на одном ядре.

| сценарий | meow | mihomo |
|---|---|---|
| DIRECT ↑ 1G | 451 Мбит/с | 423 |
| DIRECT ↓ 1G | 623 | 439 |
| DIRECT ↓ 300M, %/Мбит | 0.45 | 0.29 |
| hy2 ↑ 300M (до правки) | 0.4 | 95 |
| hy2 ↑ 300M / 1G (после) | 135 / 132, 1.31 %/Мбит | 95 / 96, 1.55 |
| hy2 ↓ 300M | 72, 1.14 %/Мбит | 67, 1.31 |

Правки: `UdpSession::send` при полной очереди драйвера отбрасывает датаграмму вместо `Closed` (иначе
TUN-поток пересоздавал hy2-сессию → новый сокет/порт на сервере → подключённые UDP-приложения теряли
поток — главная причина); BBRv2 (`Bbr2Gcongestion`, как у клиентов Hysteria) вместо CUBIC; QUIC-пакеты до
1452 Б с PMTUD (у quiche по умолчанию 1200 — датаграмма 1200 Б резалась на 2 фрагмента).
Не сделано: offload/GSO-GRO в TUN (tun-rs `offload`, `recv_multiple`/`send_multiple`), пакетная
отправка/GSO в драйвере hy2, лишние копии на пути датаграммы (oneshot + 3 аллокации на пакет).

## Дашборды (2026-11-02, `bench/dashcheck.mjs`)

zashboard и metacubexd — отдельные пакеты (`external-ui`), в ядро не встраиваются. Прогон обоих в headless
Firefox против meow и mihomo на одном конфиге. Было: zashboard `GET /storage/zashboard` → 404 (синхронизация
настроек); `/rules` с типами `RULE-SET`/`DOMAIN-SUFFIX`, payload логики `udp AND x`, без `extra` (нет
тумблеров и счётчиков); нет `unified-delay` в `/configs`; HTTP-прокси не считал заголовок запроса в upload.
Сделано как в mihomo: `/storage/{key}` (GET/PUT/DELETE, ≤1 MB, `ui-storage.json` в home), имена типов
`RuleType::mihomo_name` (в `/rules`, `rule` соединений и логах), payload `((Network,udp) && (…))`,
`extra` + `PATCH /rules/disable` (отключённое правило пропускается как PASS-цель; `RuleStats` в `RouteTable`).
Остаётся: `missCount` всегда 0 (IR не обходит все правила); порядок GLOBAL (основной исходящий первым —
решение upstream); `lazy: false` по умолчанию (ADR-0002) — mihomo проверяет группы лениво.

## TUN `stack: system` (2026-11-02)

TCP через TUN в lwIP: 685/673 Мбит/с (ul/dl), 0.22–0.24 %CPU/Мбит; mihomo (system + gso) 2490/2180, 0.05–0.06.
Сделан system stack как у mihomo/sing-box (`tun/system.rs`): NAT в помпе (C:c→D:d ⇒ B:n→A:l, ответ обратно,
in-place, RFC 1624), ядерный `TcpListener` на адресе TUN, peek-sniff (`handle_tcp_socket`), DIRECT — splice.
Итог: 2110/1680 Мбит/с, 0.07 %/Мбит. Затем offload (`IFF_VNET_HDR`, `tun/vnet.rs`): NAT переписывает
TSO/GRO-агрегаты до 64 КБ целиком (partial csum — правка только адресов), lwIP получает сегменты через
`gso_split`. Итог: **6090/5810 Мбит/с, 0.02–0.03 %/Мбит** (mihomo 2490/2180). UDP DIRECT 300M: meow 0.44–0.46
%/Мбит, mihomo 0.29. Затем UDP мимо lwIP (`tun/direct_udp.rs`: помпа разбирает IP/UDP и строит ответы,
v6/v4 csum через NEEDS_CSUM): 300M **0.31/0.33 %/Мбит**; 900M ul 537 / dl 877 Мбит/с (mihomo 427 / 475).
