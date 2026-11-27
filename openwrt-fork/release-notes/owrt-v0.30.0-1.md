## Из апстрима meow-rs (39 коммитов, по #1016 / v0.30.0)

- **Новые протоколы/инбаунды:** userspace WireGuard (`type: wireguard`, boringtun + smoltcp) и WireGuard-листенер, AnyTLS-сервер (много пользователей, v1/v2, UoT v2), `type: dns` (ответы из внутреннего резолвера), rematch-аутбаунд и `REMATCH-NAME`, process regex/wildcard и условный `SUB-RULE`.
- **API:** HTTPS/unix/pipe-контроллер с CORS и рядом исправлений безопасности (секрет через приватный файл, запрет анонимного браузерного доступа, приватные файлы на Unix). ```listeners``` получили ACL и per-listener `proxy:`/`rule:`/```routing-mark```, поименованный `type: tun`, флаг неизвестных ключей.
- **Hysteria2:** K-way пул QUIC-соединений (`quic-connections`), `dialer-proxy` для QUIC-датограмм, общий h2 для DoH, Brutal-лимиты в smux.
- **Прочее:** `global-ua`/`global-client-fingerprint`, `tcp-concurrent`, `find-process-mode`, NTP-синхронизация протокольных меток времени, сохранение выбора в профилях, младшие MSRV 1.91, чистка RUSTSEC-зависимостей, zstd без legacy-декодеров и подавление C/C++ unwind-таблиц.
- Наши DoH h2, условный SUB-RULE и правки размера ушли в апстрим раньше и вернулись доработанными.

## Поведение форка (сохранено)

- **`tproxy-port` — настоящий mihomo TPROXY** (TCP + IPv4 UDP, правила деплоера), а не управляемый REDIRECT апстрима (meow-rs#901); шорткат `redir-port` тоже остался.
- **WireGuard по умолчанию — kernel-модуль** (`wireguard-kernel`, Linux, с `amnezia-wg-option`). Userspace-клиент апстрима собран как отдельная фича `wireguard` и не включён в роутерные профили: он не нужен на слабом железе и тянет smoltcp.
- **mimalloc остаётся опцией сборки** (`alloc-mimalloc`), не включается на musl: на роутере 34–50 МБ RSS против ~5 у musl malloc.
- DNS-сервер отдаёт все адреса семейства (до 8 записей), GLOBAL-селектор упорядочен по mihomo (DIRECT, REJECT, прокси, группы). Узел, который не распарсился, остаётся «мёртвым» плейсхолдером (72a5c188), а не исчезает.
- `full-be` не содержит BoringSSL-частей (`api-tls`, `listener-anytls`, `restls`, `jls`, `dns-quic`) — big-endian MIPS собирается на rustls+quinn. В `full` эти части есть.
- **mips32-совместимость:** 0.30.0 использовал 64-битные атомики в `protocol_clock`, NTP, селекторе, dialer-bridge и API; переведены на `AtomicU`/`Uint`/`Mutex` — сборки mipsel/mips снова работают.
- `build.sh`: `-fno-unwind-tables` для всех архитектур, `-fno-exceptions` для C++, исправлен PGO-путь при абсолютном `TGT`.

## Проверено

- `cargo check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace --lib` (18/18 бинарей), ключевые integration-тесты (rules/config/api/tun/offline/persistence и др.) — зелёные.
- Размеры (stripped): aarch64 tiny 7,18 МБ (+0,33), lightweight 8,16 (+0,46), full 11,59 (+0,86); mipsel tiny 9,99 МБ (+0,72) — в 16 МБ флеш по-прежнему влезает после сжатия.
- Хост-бенч (x86_64, интерливированные прогоны): throughput ~10,4 Гбит/с и latency/conns/DNS — на уровне 0.29.0; RSS idle 15,8 → 16,7 МБ. Замеры на BPI-R4 (tunbench, hy2/REALITY) для этой версии ещё не делались.

## From upstream meow-rs (39 commits, through #1016 / v0.30.0)

- **New protocols/inbounds:** userspace WireGuard (`type: wireguard`, boringtun + smoltcp) and a WireGuard listener, AnyTLS server (multi-user, v1/v2, UoT v2), `type: dns` outbound answering through the in-process resolver, rematch outbound and `REMATCH-NAME`, process regex/wildcard rules and conditional `SUB-RULE`.
- **API:** TLS/unix/pipe controller endpoints with CORS plus security fixes (secret through a private file, no anonymous browser access, private files on Unix). Named listeners gained LAN ACLs, per-listener `proxy:`/`rule:`/`routing-mark`, named `type: tun`, unknown-key warnings.
- **Hysteria2:** K-way QUIC connection pool (`quic-connections`), `dialer-proxy` for QUIC datagrams, shared h2 for DoH, Brutal rate caps in smux.
- **Other:** `global-ua`/`global-client-fingerprint`, `tcp-concurrent`, `find-process-mode`, NTP-adjusted protocol clocks, profile selection persistence, MSRV 1.91, RUSTSEC dependency cleanup, zstd without legacy decoders, C/C++ unwind-table suppression.
- Our DoH h2, conditional SUB-RULE and footprint changes had gone upstream earlier and came back reviewed.

## Fork behaviour (kept)

- **`tproxy-port` is mihomo's real TPROXY** (TCP + IPv4 UDP, deployer-owned rules), not upstream's managed REDIRECT (meow-rs#901); `redir-port` is kept too.
- **WireGuard defaults to the kernel module** (`wireguard-kernel`, Linux, `amnezia-wg-option`). Upstream's userspace client is a separate `wireguard` feature, off in router profiles — it pulls smoltcp and buys nothing on small hardware.
- **mimalloc stays opt-in** (`alloc-mimalloc`): 34–50 MB of RSS on a router against ~5 with musl malloc.
- The DNS server returns every address of the family (up to 8 records), GLOBAL is ordered as in mihomo (DIRECT, REJECT, proxies, groups), and a node that fails to parse stays a dead placeholder (72a5c188).
- `full-be` carries no BoringSSL parts (`api-tls`, `listener-anytls`, `restls`, `jls`, `dns-quic`) so big-endian MIPS builds on rustls+quinn; `full` has them.
- **mips32 compatibility:** 0.30.0 used 64-bit atomics in `protocol_clock`, NTP, the selector store, the dialer bridge and the API; they now use `AtomicU`/`Uint`/`Mutex`, so mipsel/mips build again.
- `build.sh`: `-fno-unwind-tables` on every arch, `-fno-exceptions` for C++, and a fixed PGO path for absolute `TGT`.

## Tested

- `cargo check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace --lib` (18/18 binaries), key integration suites (rules/config/api/tun/offline/persistence, etc.) — green.
- Stripped sizes: aarch64 tiny 7.18 MB (+0.33), lightweight 8.16 (+0.46), full 11.59 (+0.86); mipsel tiny 9.99 MB (+0.72) — still fits 16 MB flash after compression.
- Host bench (x86_64, interleaved runs): ~10.4 Gbps throughput and latency/conns/DNS on par with 0.29.0; idle RSS 15.8 → 16.7 MB. BPI-R4 (tunbench, hy2/REALITY) numbers for this version are not measured yet.
