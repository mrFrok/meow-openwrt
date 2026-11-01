# BENCHMARKS — Mihomo Lightweight

Правило: в каждой строке указаны **железо, версия OpenWrt и ядра, Go, профиль/теги, конфиг**. Эмуляция и десктоп помечаются явно и не выдаются за роутер.

## Методика (§16)
- **Сравниваем:** upstream v1.19.33 · Lightweight · Lightweight + fast path (D5) · Lightweight + sockmap (D6, если дойдёт) · fast path + sockmap.
- **Сценарии:** простой; 10/100/500/1000 соединений; TCP DIRECT/PROXY; UDP DIRECT/PROXY; DNS (QPS, задержка); TUN, TPROXY, redirect; короткие (wrk) и длинные (iperf3 -t 60) соединения; большой набор правил (geosite + rule-set по 100 тыс. записей).
- **Метрики:**
  - размер (stripped и xz);
  - RSS/Anon/File/Pss;
  - память ядра (Slab, SUnreclaim, PageTables);
  - CPU ядра-прокси (% одного ядра) и softirq;
  - горутины (API `/debug` не используем — `/connections` и счётчик потоков);
  - паузы GC (GODEBUG=gctrace=1);
  - пропускная способность, pps, задержка, джиттер, потери (iperf3 -u).
- **Инструменты:**
  - на роутере: `bench/baseline.sh <label> <секунды>` — CSV раз в секунду;
  - на ПК в LAN: iperf3, wrk, netperf;
  - на десктопе: `bench/size.sh`, `bench/mem.sh`.
- **Безопасность** (проверяется до замеров скорости, §18):
  - PROXY-домен на общем с DIRECT IP не уходит мимо прокси;
  - при смене правил старые обходы исчезают (conntrack и flowtable);
  - при падении ядра: нет утечки PROXY и нет DNS-утечек;
  - IPv6 ведёт себя так же, как IPv4.

## Результаты
### Десктоп x86_64 (не роутер), Go 1.27.3, 2026-11-01
Размер stripped, байты (`bench/size.sh`):

| профиль | arm64 | armv7 | mipsle-sf | mips-sf | amd64 v1 |
|---|---:|---:|---:|---:|---:|
| official (`with_gvisor`) | 56 492 156 | 55 967 868 | 62 783 643 | 62 783 643 | 61 255 804 |
| lean | 37 027 964 | 37 552 252 | 42 729 627 | 42 729 627 | 40 947 836 |
| ultra-trial (только case'ы, без gvisor) | 28 901 500 | — | — | — | 32 194 684 |

Ultra-trial arm64: gzip -9 10.7 МБ, xz -9 7.3 МБ. Секции: `.text` 12.0 МБ, `.gopclntab` 10.4 МБ, `.go.type` 3.2 МБ.

RAM, КБ (`bench/mem.sh`, amd64, пустой конфиг `MATCH,DIRECT`, GOMAXPROCS=1, бинарник в tmpfs):

| сборка | RSS idle | Anon | File | RSS при 100 закачках | Anon |
|---|---:|---:|---:|---:|---:|
| official | 44 700 | 12 700 | 32 000 | 48 640 | 15 060 |
| lean | 34 632 | 10 020 | 24 612 | 38 836 | 12 900 |
| ultra-trial | 29 964 | 9 604 | 20 360 | 34 412 | 12 856 |

GOMAXPROCS=4: разница меньше 2 МБ.

### BPI-R4 (MT7988A, 4× Cortex-A73 1.8 ГГц, AES/PMULL/SHA, 4 ГБ RAM + zram/swap 2 ГБ), 2026-11-01
Своя сборка OpenWrt 25.12-SNAPSHOT r33324 (ядро 6.12.112) с патчами MediaTek. Отличия от generic: CGROUPS/CGROUP_BPF=y, IO_URING=y, NF_FLOW_TABLE=y; NET_SOCK_MSG=y, BPF_STREAM_PARSER нет; TLS нет. HW flow offload включён (`flags offload`, PPE), две flowtable (`ft` от fw4 и `f`), установлен kmod-mediatek_hnat. Бинарник ядра лежит в /usr/bin на overlay (f2fs), так что страницы файла вытесняемы.

**Боевое ядро** (upstream v1.19.33, go1.26.10, 57.5 МБ, боевой конфиг clashwrt, режим с `clash-tun`, `GOMEMLIMIT=64MiB`): VmRSS 95 124 КБ, **RssAnon 57 236 КБ**, RssFile 37 888 КБ, 11 потоков. Для сравнения: живой heap ~8–9 МБ (gctrace, 2026-10-31). Гипотеза: большая часть anon — запас GC под лимит 64 МиБ. Проверка — `bench/livemem.sh`.

**D4, эксперимент 2026-11-01** (`bench/livemem.sh`, боевой конфиг, GOMEMLIMIT=64MiB, 90 с без трафика, по одному прогону, бинарники arm64 Go 1.27.3 с `with_gvisor`, запуск из /tmp, поэтому RssFile≈0 — страницы в shmem):

| сборка | endAnon, КБ | живой heap | пик heap на старте |
|---|---:|---:|---:|
| base | 30 352 | 6 МБ | 42 МБ |
| lw_serial_load | 41 860 | 6 МБ | 41 МБ |
| **lw_free_after_load** | **18 028** | 6 МБ | 42 МБ |
| оба | 20 524 | 6 МБ | 31 МБ |

Предыдущий прогон с upstream-бинарником (go1.26.10): GOMEMLIMIT 64→32 МиБ при GOGC=100 anon не меняет (~35 МБ); 32 МиБ + GOGC=50 даёт 17 МБ.
Вывод: anon держит пик загрузки rule-set, а не живые данные. Последовательная загрузка не помогает (тег убран). FreeOSMemory после ApplyConfig: −41%.

**D4, сравнение upstream и Lightweight, 2026-11-01** (`bench/livemem.sh`, боевой конфиг, 90 с без трафика, по одному прогону). Upstream — боевой /usr/bin/mihomo (v1.19.33, go1.26.10, 57.5 МБ, f2fs). Lightweight — arm64 Go 1.27.3, 35.1 МБ, запуск из /tmp (RssFile≈0, бинарник в shmem), теги `with_gvisor no_tailscale no_zerotier no_easytier with_low_memory no_fake_tcp no_pprof no_restls no_jls no_kcptun lw_free_after_load lw_auto_memlimit`. `X=1` = запуск без GOMEMLIMIT/GOGC (у Lightweight тогда авто: 48 МиБ при 4 ГБ, GOGC=50).

| вариант | upstream endAnon, КБ | Lightweight endAnon, КБ | живой heap up / lw |
|---|---:|---:|---:|
| без env (X=1) | 37 116 | **15 772** | 6 / 5 МБ |
| GOMEMLIMIT=64MiB | 37 652 | 26 912 | 6 / 5 МБ |
| GOMEMLIMIT=32MiB, GOGC=50 | 15 856 | 14 812 | 6 / 5 МБ |

Выводы: авто-режим Lightweight (48 МиБ + GOGC=50) почти равен 32 МиБ + GOGC=50 (15.8 против 14.8 МБ) — опускать лимит до 32 смысла нет, решает GOGC=50. Без env: −21 МБ anon (−57%). 64 МиБ у Lightweight (26.9 МБ) расходится с прошлым замером lw_free_after_load (18.0 МБ) — один прогон, повторить. Живой heap −1 МБ.

### Устройства добровольца (hwprobe.sh, 5 роутеров, 2026-11-01)
Один человек, сырые выводы — `research/hardware/hwprobe-2026-11-01/`. Ядро прокси ни на одном не запущено (секция `proxy-core` пустая), поэтому RSS нет — только железо и система. **MIPS среди них нет**, данных по mt7621/ath79 по-прежнему нет.

| устройство | SoC / ядра | RAM | ПЗУ, свободно | OpenWrt / ядро | заметное |
|---|---|---:|---|---|---|
| Cudy WBR3000UAX | MT7981, 2× A53 | 485 МБ | NAND ubifs, 80 МБ | 25.12.5 / 6.12.94 (stock) | нет kmod-tun, kmod-nft-tproxy/socket; dnsmasq не full; `hw-tc-offload: on` |
| Xiaomi Redmi AX6000 | MT7986, 4× A53 | 484 МБ | NAND ubifs, 80 МБ | 25.12.5 / 6.12.94 (stock) | то же, что у Cudy |
| NanoPi R3S | RK3566, 4× A55 1.8 ГГц | 1.9 ГБ | ext4, 199 МБ | 25.12.6 / 6.12.112 | tun, tproxy, socket, wireguard, amneziawg |
| NanoPi R76S | RK3576, 4× A53 + 4× A72 2.2 ГГц | 1.9 ГБ | ext4, 199 МБ | 25.12.6 / 6.12.112 | как R3S |
| NanoPi R6S | RK3588S, 4× A55 + 4× A76 1.8 ГГц | 7.8 ГБ | ext4, 199 МБ | 25.12.6 / 6.12.112 | как R3S |

У всех: aarch64, ARMv8 Crypto Extensions (`aes pmull sha1 sha2`), то есть в Go работает ассемблерный AES-GCM; LSE-атомики (`atomics`) — только у A55/A76 (R3S, R6S), у A53/A72 нет (Go выбирает путь в рантайме). kmod-tls есть везде. bpf_jit=1, cgroup2, bpffs смонтирован. Своп нет нигде.

### Устройства, второй пакет (hwprobe.sh, 4 роутера, 2026-11-01)
Сырые выводы — `research/hardware/hwprobe-2026-11-01b/` (файлы присланы как `R4AV2`, `Cudy`, `wr840v6`, `wr841v8`; `wr840v6` по содержимому — Keenetic Extra II). Ядро прокси нигде не запущено. **Первые MIPS.**

| устройство | SoC / ядра | RAM, доступно | ПЗУ, свободно | OpenWrt / ядро | заметное |
|---|---|---:|---|---|---|
| Xiaomi Mi 4A Gigabit v2 | MT7621, MIPS 1004Kc 880 МГц, 2 ядра × 2 потока, без FPU | 116 МБ, **42 МБ** | jffs2, 53 МБ | 25.12.6 / 6.12.112 | нет kmod-tun и nft_tproxy (только redir); EIP-93 (`kmod-crypto-hw-eip93`) |
| Cudy WBR3000UAX (другой экземпляр) | MT7981, 2× A53 | 485 МБ, 258 МБ | ubifs, 45.8 МБ | 25.12.5 / 6.12.94 | tun, tproxy, safexcel; zapret2, AmneziaWG |
| Keenetic Extra II | MT7628AN, MIPS 24KEc 580 МГц, 1 ядро | 57 МБ, **14.8 МБ** | jffs2, **6.2 МБ** | 24.10.8 / 6.6.144 | нет tun/tproxy; zapret |
| TP-Link TL-WR841N v8 | AR9341, MIPS 74Kc (big-endian), 1 ядро | 57 МБ, **13 МБ** | jffs2, **3.1 МБ** | 24.10.0 / 6.6.73 (ath79/tiny) | zram; уже стоят Podkop, zapret, AmneziaWG |

Размер ядра под mipsle softfloat (HEAD, `-s -w`): tiny **27.3 МБ** (xz 5.8 МБ), lightweight 34.8 МБ (xz 6.9 МБ) — на MIPS код больше, чем на arm64 (tiny arm64 — 23.5 МБ).
Разбор tiny mipsle (2026-11-01): символы — 16.7 МБ из 27.3 (остальное — pclntab и метаданные типов, растут с числом функций). Крупнее всего: mihomo 2.4 МБ, std crypto 1.6, runtime 0.8, mipstack 0.68, utls 0.67, net 0.66, quic-go 0.66, metacubex/http 0.59, tls 0.43, x/net 0.37, miekg/dns 0.35, sing-tun 0.31, klauspost/compress 0.30 (нужен utls), x/text 0.22 (idna), regexp2 0.17 (фильтры групп/провайдеров), bbolt+msgpack 0.27 (cachefile). Все существующие теги отключения в tiny уже стоят; новые теги дали бы по 0.1–0.4 МБ ценой функций. `crypto/internal/fips140/drbg.memory` — 32 МБ BSS, но трогается только в режиме FIPS 140-3, RSS не занимает.
- `-gcflags=all=-l`: **24.6 МБ** (−2.75 МБ, −10%), xz 4.7 МБ (−1.06). `-B` ещё −0.46 МБ, но убирает проверки границ — не использовать. Цена -l по CPU на MIPS неизвестна (arm64 TCP +4%) — мерить скриптом у добровольца.
- Тег `lw_no_mipstack` (sing-tun без mipstack; стек по умолчанию `mips` → `system`): 27.3 → **26.1 МБ** (−1.25), xz 5.49. Вместе с `-l`: **23.4 МБ**, xz 4.49. Включён в tiny вместе с `profiles/tiny.gcflags` (`all=-l`) — решение пользователя 2026-11-01: tiny mipsle **23.4 МБ**, gz 6.7, xz 4.3.
- Ещё в бинарнике два HTTP/TLS-стека: std `net/http` 0.33 + `crypto/tls` 0.32 МБ символов рядом с форками metacubex — std тянут x/net/http2, gobwas/ws, sing-mux, miekg/dns (DoT), vless vision; убрать — переписывать зависимости, не окупается.
- Оценка `no_quic` (без Hysteria2, DoQ, DoH3, QUIC-сниффера, xhttp-h3): символы quic-go+http3+qpack 0.67 МБ + sing-quic 0.11 МБ; по соотношению mipstack (0.68 МБ символов → −1.25 МБ бинарника) — **≈ −1.4 МБ**, xz ≈ −0.3 МБ. Для tiny не делаем — Hysteria2 обязателен.
- Свой код mihomo — 2.4 МБ символов, размазан ровно (крупнейшая функция `ConvertsV2Ray` 45 КБ); generic-инстанциации 0.69 МБ на весь бинарник (xsync.Map 62 КБ, LruCache 31 КБ) — переписывание даст ~0.1 МБ, не окупается.
- Боевой mihomo на BPI-R4 (бинарник 57 МБ): RssFile 37.5 МБ — без нехватки памяти страницы кода не вытесняются; рабочий набор под давлением не измерен (ядро без smaps).
- **Mi 4A Gigabit (MT7621)** — целевое MIPS-устройство: ПЗУ хватает, но распаковка в tmpfs съела бы 27 из 42 МБ доступной RAM. Держать ядро распакованным на jffs2 (страницы кода тогда вытесняемые), а не в /tmp; для UDP нужен `kmod-tun` (tproxy тоже нет). Замеры MIPS (gcflags, ChaCha vs AES, EIP-93) — только скриптом, который владелец запускает сам (SSH не будет).
- Пользователь уточнил: оба TP-Link (WR841N v8 и «wr840v6») **модифицированы** — ПЗУ/ОЗУ 16/64 МБ вместо родных 4/32; у Xiaomi ПЗУ заменено с 16 на 64 МБ. На **родном Mi 4A Gigabit (16 МБ ПЗУ)** свободно обычно ~8 МБ (пользователь): ядро на флеш не ляжет — xz 5.8 МБ (4.7 с `-l`) влезет впритык вместе с обвязкой, распакованное 27 МБ — нет → только tmpfs → 27 из 42 МБ свободной RAM: на стоковом MT7621 со 128 МБ ядро работает впритык (из tmpfs код не вытесняется), нужен tiny меньшего размера. Родные 4/32 TP-Link — тем более мимо.
- **Keenetic Extra II, WR841N v8** — Go-ядро не помещается: свободной RAM 13–15 МБ меньше одного бинарника в tmpfs, ПЗУ 3–6 МБ. Это класс устройств не для mihomo (там zapret/AWG); в README честно указать минимум (≈128 МБ RAM).

Выводы для ядра:
- arm64 — основная цель вместе с MIPS; на этих устройствах шифрование не узкое место, AES-GCM быстрее ChaCha20.
- ПЗУ не тесное (≥ 80 МБ свободно): им подходит профиль lightweight, tiny нужен только для 16–32 МБ флеша.
- `lw_auto_memlimit` на всех пяти даёт 48 МиБ (MemTotal/8 ≥ 60 МБ → потолок), GOGC=50. На 485 МБ это ~10% RAM — нормально.
- На стоковом OpenWrt filogic из коробки нет kmod-tun и nft-tproxy: работает только redirect (TCP). Ядро при `tun: enable: true` без `/dev/net/tun` должно давать понятную ошибку — проверить текст ошибки (общее для всех обвязок).

### Размеры профилей после сужения протоколов (2026-11-01)
`lightweight/build.sh`, stripped, Go 1.27.3; полная таблица — `profiles/README.md`. tiny: arm64 25.69 → **24.19** МиБ, mipsle-sf 29.81 → **28.13** МиБ (xz 5.91). lightweight: arm64 33.50 → **26.56**, mipsle-sf 38.69 → **30.69** МиБ. Вклад тегов в tiny arm64 (по сумме секций ELF): no_vmess ≈ 393 КБ, серверные inbound + легаси ≈ 0.6–1.1 МБ, anytls 91 КБ, ssr 72 КБ, gost-relay 31 КБ; socks5+http outbound стоят 38 КБ (оставлены). Userspace WireGuard/AmneziaWG стоит 917 КБ в tiny (только в full).

### tiny: no_bundle и инлайнинг (2026-11-01, arm64 stripped)
`no_bundle` (tiny, lightweight): 23 920 764 → **23 462 012** байт (−448 КБ, 22.38 МиБ). HTTP/3 из tiny не убрать: `http3` импортирует сам Hysteria2 (`sing-quic/hysteria2`); std `crypto/tls` (224 КБ) держат net/http, miekg/dns, gobwas/ws, vision, sing-mux.

| сборка tiny | arm64, байт | МиБ | mipsle-sf, байт |
|---|---:|---:|---:|
| как есть | 23 462 012 | 22.38 | 27 328 667 |
| `-gcflags=all=-l -gcflags=std=` (инлайнинг только в std) | 22 020 220 | 21.00 | — |
| `-gcflags=all=-l` (без инлайнинга) | 21 102 716 | 20.13 | 24 510 619 |

Цена по CPU (TCP Reality, BPI-R4): `all=-l` — +4% CPU на отдаче, см. «Этап IV: TCP через Reality»; MIPS не замерен. Горячий путь шифрования — ассемблер (AES-GCM, ChaCha20) и std, вариант «инлайнинг только в std» его не трогает.

### Память при обновлении rule-провайдеров (2026-11-01, `bench/provmem.sh`, BPI-R4)
Боевой конфиг, принудительное обновление `refilter_domains` и `ru_bundle` через API, `GOMEMLIMIT=48MiB GOGC=50` у всех. RssAnon, МБ:

| бинарник | до | пик | через 30 с |
|---|---:|---:|---:|
| upstream v1.19.33 | 25.8 | 30.8 | 30.8 |
| lightweight без `lw_free_after_load` | 32.3 | 32.3 | 32.3 |
| lightweight с `lw_free_after_load` | **19.6** | **20.1** | **20.1** |

`lw_free_after_load` держит обновление почти без всплеска (+0.5 МБ против +5 у upstream) и экономит ~12.7 МБ против той же сборки без хука. Бинарники lightweight — до сужения протоколов (arm64, 32.8 МБ). Один прогон.

### lw_noproto и цена Mieru (2026-11-01)
`lw_noproto` (во всех профилях): `geosite.dat`/`geoip.dat` и VLESS addons разбираются ручным декодером на `protowire` вместо рантайма protobuf. На реальных файлах (1499 geosite, 217 946 доменов, 6827 атрибутов, 260 geoip, 1 188 681 CIDR) дамп побайтно совпадает с protobuf-версией, разбор 0.25 с против 0.32 с, пик RSS 146 МБ против 223 МБ (x86-64, оба файла целиком). tiny: arm64 24.19 → **22.81** МиБ (−1.4 МБ), mipsle-sf 28.13 → **26.63** МиБ (−1.5 МБ), xz mipsle 5.65 МиБ. В lightweight и full эффекта нет: protobuf (с protojson) тянет библиотека Mieru (`enfein/mieru/v3`: metricspb, appctlpb, trafficpattern). Mieru стоит lightweight arm64 **2.1 МБ** (32 768 124 → 30 539 900 с `no_mieru`): ≈0.75 МБ сам протокол + ≈1.4 МБ protobuf. Решение пользователя: Mieru убран из lightweight (`no_mieru`), есть только в full. lightweight: arm64 31.25 → **29.13** МиБ, mipsle-sf 36.13 → **33.69** МиБ (xz 6.75).

| профиль | arm64 | armv7 | mipsle-sf | mips-sf | amd64 | xz mipsle |
|---|---:|---:|---:|---:|---:|---:|
| tiny | 22.81 | 23.44 | 26.63 | 26.63 | 24.75 | 5.65 |
| lightweight | 29.13 | 29.69 | 33.69 | 33.75 | 31.43 | 6.75 |
| full | 33.06 | 33.50 | 38.00 | 38.00 | 36.55 | 7.64 |

### Baseline upstream на BPI-R4 (2026-11-01, `bench/baseline-run.sh upstream`)
Боевое ядро upstream v1.19.33 (go1.26.10, `GOMEMLIMIT=64MiB`), режим clashwrt `redirect_tun` (TCP — redirect на :7893, UDP — TUN), на время прогона `bypass_set_mode=off`, watchdog выключен; узлы закреплены: TCP — `🇵🇱 TCP Reality` (VLESS+REALITY), UDP — `🇵🇱 Hysteria2`; оба на том же VPS, что и iperf3 (217.60.33.169). Источник — ПК в LAN (1 Гбит/с), в Throne правило direct для VPS. iperf3 по 27 с, CPU и память — `bench/baseline.sh` раз в секунду, среднее без первых 2 с. core% — % одного ядра у процесса mihomo, sys% — занятость всех 4 ядер, sirq% — softirq.

| сценарий | Мбит/с | core% | sys% | sirq% | RssAnon, МБ |
|---|---:|---:|---:|---:|---:|
| простой | — | 0 | 4 | 0 | 51 |
| TCP DIRECT ↓, 1 поток | 341 | 20 | 15 | 5 | 51 |
| TCP DIRECT ↑, 1 поток | 352 | 6 | 10 | 4 | 62 |
| TCP DIRECT ↓, 4 потока | 721 | 15 | 20 | 10 | 62 |
| TCP PROXY (Reality) ↓, 1 поток | 379 | 29 | 18 | 5 | 60 |
| TCP PROXY (Reality) ↑, 1 поток | 289 | 13 | 11 | 3 | 60 |
| UDP DIRECT ↓ (300M, через TUN) | 286 (потери 4.8%, джиттер 0.06 мс) | 83 | 32 | 6 | 60 |
| TCP PROXY ↓, 4 потока | **не снят**: 4 одновременных dial к узлу Reality (sportzal:443) — `context deadline exceeded`, похоже на лимит новых соединений на VPS | | | | |
| UDP PROXY (Hysteria2) ↓ | **не снят**: управляющее TCP-соединение iperf3 (через Reality) получает тот же таймаут dial, напрямую 5204 работает | | | | |

Выводы (один прогон): TCP упирается не в роутер (CPU ядра ≤ 30% одного ядра при 340–380 Мбит/с, 4 потока DIRECT — 721 Мбит/с), вероятно в канал/VPS. UDP через TUN — самый дорогой путь: 83% одного ядра на 286 Мбит/с и 4.8% потерь — первый кандидат для этапа IV. Anon у upstream растёт с 51 до 62 МБ под нагрузкой (64 МиБ лимит). Сырые данные — `out/baseline/upstream/` (не в git).

## Этап IV: UDP через TUN — recvmmsg + UDP GRO (2026-11-01)

Стенд без VPS: iperf3 в netns на самом роутере (`bench/netns-iperf.sh up`, 203.0.113.2 за veth; канала провайдера в пути нет), `VPS=203.0.113.2 bench/prof-run.sh <label> udp-direct-dl <бинарник>`; сборка tiny + pprof, arm64, BPI-R4. На ПК-источнике Throne должен пропускать 203.0.113.0/24 напрямую (или быть выключен).

Профиль до правки: 70% CPU — syscalls, один `recvmsg` и один `write()` в TUN на каждую датаграмму (`systemUDPPacketWriter4.WritePacket` → `NativeTun.Write` → `BatchWrite` из одного пакета; UDP GRO в sing-tun получал по пакету).
Правка: `tunnel/udpbatch_lw_linux.go` — `recvmmsg` до 16 датаграмм за вызов для простого UDP-сокета (DIRECT), буферы из пула только когда сокет готов; пакетная запись (`constant.WriteBackBatch` → `listener/sing` → форк sing-tun `WritePacketBatch`); пакет, с которого начинается серия, получает ёмкость под всю серию, иначе GRO отказывает (`coalesceInsufficientCap`).

| сборка | -b | доставлено, Мбит/с | потери | core% |
|---|---:|---:|---:|---:|
| tiny, по пакету | 1G | ~303 | 61% | 102 |
| tiny, recvmmsg без склейки | 1G | ~407 | 47% | 106 |
| **tiny, recvmmsg + GRO** | 1G | **~738** | **1.5%** | 118 |
| tiny, по пакету | 300M | 297 | 0.83% | 90 |
| tiny, recvmmsg + GRO | 300M | 299 | 0.09% | 80 |

Доставлено = отправлено × (1 − потери). На 300 Мбит/с CPU почти не меняется: `recvmmsg` успевает забрать по одному пакету, склеивать нечего.

## Этап IV: TCP через Reality — профиль CPU и `-gcflags` (2026-11-01)
Стенд без VPS: в netns роутера (`bench/netns-iperf.sh up`) — iperf3 на 5201–5214 и сервер VLESS+Reality+Vision (`bench/reality-server.yaml`, сборка с серверными inbound, `taskset` на CPU 3; dest Reality — собственный TLS-контроллер сервера, интернет не нужен). Клиент — tiny с pprof в режиме clashwrt `redirect_tun`, узел вставляется `PROXY_INJECT=bench/reality-client.yaml TCP_TARGET="LW Reality"`. Источник — ПК в LAN, поэтому потолок 940 Мбит/с (1 Гбит/с LAN), все прогоны его достигают; сравнение — по CPU. core% — % одного ядра у клиентского mihomo (сервер в netns — отдельный процесс, sys% его включает). Запуск: `VPS=203.0.113.2 TCP_TARGET="LW Reality" PROXY_INJECT=bench/reality-client.yaml bench/prof-run.sh <label> tcp-proxy-dl4`.

| tiny, 940 Мбит/с | core% (2 круга) | sys% | профиль |
|---|---:|---:|---|
| DIRECT ↑ | 51.7 | 39 | syscalls 79% |
| Reality ↓, 4 потока | 32.4 / 33.9 | 27 | syscalls 46% (read 29, write 18), AES-GCM 33%, memmove 5% |
| Reality ↑, 1 поток | 68.6 / 69.6 | 54 | syscalls 56% (read 28, write 30), AES-GCM 22%, memmove 2% |
| Reality ↓, `-gcflags=all=-l` | 33.4 / 33.3 | 28 | — |
| Reality ↑, `-gcflags=all=-l` | 72.6 / 71.4 | 54 | — |

- Reality на BPI-R4 — не узкое место: гигабит стоит ⅓ ядра на приём и ~0.7 ядра на отдачу. Надбавка к DIRECT ↑ (+17 п.п.) — это почти целиком AES-GCM (аппаратный, ARMv8 AES/PMULL); остальное — syscalls read/write, как у DIRECT. kTLS здесь мог бы убрать только копирования в user space, а не шифрование — на BPI-R4 не стоит работы (см. TODO, «шифрование в ядре»).
- `-gcflags=all=-l` (−2.36 МБ): ↓ — в пределах шума, ↑ — +3 п.п. CPU (+4%), больше разброса между кругами (~1 п.п.). По критерию TODO («потеря ≤ шума») в tiny по умолчанию не включаем; вариант `all=-l std=` и UDP Hysteria2 не замерены.

## Этап IV: UDP через Hysteria2 — стенд в netns (2026-11-01, предварительно)
Сервер — тот же netns (`bench/reality-server.yaml`, листенер `hy2` :8444), клиент — tiny с pprof, узлы `LW Hysteria2` (BBR) и `LW Hysteria2 Brutal` (`up/down: 1000 mbps`) из `bench/reality-client.yaml`, `UDP_TARGET=...`; iperf3 ↓ 300M, `UDP_LEN` — размер датаграммы.

| вариант | доставлено, Мбит/с | потери | core% клиента |
|---|---:|---:|---:|
| BBR, 1460 Б (по умолчанию iperf3) | 117 | 61% | 149–173 |
| Brutal, 1460 Б | 117 | 61% | 140 |
| BBR, 1200 Б | 96 | 68% | 56 |
| Brutal (клиент и сервер), 1200 Б | 93 | 69% | 114 |

- Предел ~90–120 Мбит/с — **тестового сервера**, не роутера: отбрасывает исходящий DIRECT-сокет сервера в netns (Recv-Q полон, `RcvbufErrors` только в netns, на хосте 0); сервер при этом занимает 0.74 ядра (2 ядра через `SRV_CPU=2,3` не помогают), от BBR/Brutal не зависит. Нагрузить клиента выше ~100 Мбит/с локально не удалось; VPS для UDP недоступен (порты закрыты).
- 1460-байтные датаграммы больше QUIC DATAGRAM — Hysteria2 режет каждую на 2 фрагмента, CPU клиента ×2–3. Реальные приложения (QUIC, голос, игры) шлют ≤1350 Б, поэтому мерить нужно с `UDP_LEN=1200`.
- Клиент (BBR, 1200 Б): ~0.6% ядра на Мбит/с — примерно вдвое дороже DIRECT через TUN (0.27). По профилю (1460 Б): запись в TUN по одному пакету (`WriteBack` → `NativeTun.Write`) 20%, GC 19%, приём quic-go уже пакетный (`ipv4.ReadBatch` / recvmmsg) ~9%.

### Этап IV: Hysteria2-клиент — пакетная запись в TUN (2026-11-01)

Форк sing-quic (`third_party/sing-quic`, LIGHTWEIGHT.md): датаграммы обрабатываются без горутины на каждую, заголовок разбирается/собирается без `binary.Read/Write`, строка назначения и `net.Addr` переиспользуются; `udpPacketConn.ReadPacketBatch` забирает всю очередь, `tunnel/udpbatch_proxy_lw.go` пишет её в TUN одним `WriteBackBatch`. Deadline-обёртка (`common/net/deadline`) мешала добраться до conn — каждая отправка ставит read deadline; `DisablePipeLW` переводит её на прямые дедлайны.

Стенд netns, `udp-proxy-dl`, 1200 Б, iperf3 300M, A/B одной сборкой full-тегов + pprof, по 3–4 прогона:

| Клиент | Принято, Мбит/с | Ядро mihomo | %/Мбит |
|---|---|---|---|
| до | 108 | 158–161% | 1.46–1.49 |
| после | 116–119 | 127–130% | **1.09–1.10** (−26%) |

Один прогон «после» дал 132 Мбит/с при 104% (0.79) — не повторился, считать выбросом. Сервер, пересобранный с тем же форком (быстрее разбор/сборка заголовка), потолок стенда не поднял (118 Мбит/с) — упор не в sing-quic.

#### Сервер на ноутбуке в LAN (Wi‑Fi 7, 6 ГГц) — стенд без потолка netns

Ноутбук (Core Ultra 5 225H, Artix): `~/lwsrv` — тот же `bench/reality-server.yaml` (amd64), iperf3 5201–5214, цель — `203.0.113.3/32` на `lo` ноутбука (адрес вне LAN, иначе ПК ходит к нему мимо роутера). Клиентский узел — `bench/reality-client.yaml` с `server: <IP ноутбука>`; `VPS=203.0.113.3`. Тестовый netns на роутере выключить (`netns-iperf.sh down`), иначе baseline.sh меряет CPU не того mihomo. На ПК Throne (TUN) должен пускать 203.0.113.0/24 в direct. Wi‑Fi сам по себе: UDP ноутбук→ПК 468 Мбит/с, TCP 742.

`udp-proxy-dl`, 1200 Б, 300M:

| Клиент | Принято, Мбит/с | mihomo | %/Мбит |
|---|---|---|---|
| до | 137–139 | 212–215% | 1.52–1.57 |
| после | 240–241 | 158–162% | **0.65–0.67** (−57%) |

Старый клиент упирался сам в себя; новый — нет (~1.6 ядра из 4): при 600M и с Brutal 1000 mbps те же ~236–240, все потери — `RcvbufErrors` DIRECT-сокета **сервера** (его отправка через quic-go: `sendQueue` ~54% ядра на sendmsg, цепочка tunnel → `SendDatagram` → run loop → sendQueue не успевает, ни одна горутина не на 100%). На netns-сервере (ARM) тот же путь давал ~118 — масштабируется с CPU.

`udp-proxy-ul` (новый сценарий, отдача через Hysteria2): 200M → принято 129 Мбит/с, 35% потерь, mihomo 171%, 1.33 %/Мбит; при 600M — 84 Мбит/с (роутер 81% всех ядер, чтение TUN 45%). Потери не в ядре (UDP-счётчики роутера/ноутбука и дропы TUN — 0): пакеты выбрасываются внутри mihomo — очередь tunnel `packetSender` (`default: packet.Drop()`) и/или очереди приёма sing-quic/quic-go на сервере, потому что отправка `SendDatagram` по одному на пакет медленнее входа. Это следующий кандидат: пакетная отправка датаграмм в quic-go на клиенте.

Разбор отдачи (2026-11-01, временные счётчики): почти все потери — переполнение очереди `packetSender` клиента (128, `default: packet.Drop()`), ~6 тыс. пакетов/с при 200M; очередь sing-quic на сервере теряет сотни. Горутина `Process` занята ~20% CPU, остальное время ждёт: цепочка `Process` → `SendDatagram` → run loop → `sendQueue` будит друг друга на каждый пакет, ни одна стадия не на 100%. `sendQueue` — ~45% ядра, почти всё `sendmsg` (~27 мкс на пакет на ARM): в quic-go GSO-пачка продолжается только при `size == maxSize`, а пакет с датаграммой всегда меньше — каждая датаграмма уходит отдельным sendmsg.
- Патч quic-go «размер сегмента GSO = размер первого пакета пачки» (`research/patches/quic-go-gso-segsize.patch`, с тестами; пакер не ограничивать — он выбрасывает DATAGRAM, который не влез): отдача 121 → 131 Мбит/с, 1.35 → 1.15 %/Мбит (по одному прогону), приём без изменений — пачки почти не собираются, run loop успевает за каждым пакетом. Форк quic-go (4 МБ) ради этого не взят.
- `senderCapacity` 128 → 1024: 131 → 149 Мбит/с, потери при 200M 35% → 26% — не взято: +16 КБ на каждый UDP-поток (DNS и т. п.).
- Передача роутера по Wi‑Fi не предел: iperf3 с роутера на ноутбук 400 Мбит/с — 33% от 4 ядер вместе с самим iperf3.
- Трассировка (`/debug/pprof/trace`, `go tool trace -pprof=sync|sched|syscall`): `Process` 1.1 с из 3 стоит в `datagramQueue.Add` (очередь quic-go полна) — пакетная передача внутри mihomo не поможет; `sendQueue` 1.8 с из 3 в `sendmsg` (~40 мкс на пакет), run loop и `sendQueue` ещё ~1.7 с ждут процессора (пробуждение потоков на каждый пакет). Brutal 1000 mbps — то же (122 / 142 с патчем): не pacing.
- **Стенд «по кабелю»** (сервер Hysteria2 на ПК по Ethernet, iperf3-клиент на ноутбуке по Wi‑Fi через `IPERF_SSH`; роутер шлёт в Ethernet, как в WAN): отдача 200M — 123 → 133 Мбит/с с GSO-патчем (1.17 → 1.04 %/Мбит), 400M — 120 → 124. Потолок ~120–130 Мбит/с не от Wi‑Fi; GSO-патч даёт +5–8% — форк quic-go не окупается. Остаток — однопоточная цепочка quic-go на соединение (run loop + sendQueue) на ARM. **Решение пользователя (2026-11-01): оптимизацию quic-go не делаем** — игра не стоит свеч (поток >125 Мбит/с на отдачу редок, форк дорог в поддержке).

Остаток по профилю: запись в TUN (write(2)) 15%, приём quic-go (recvmmsg + расшифровка) ~25%, GC 13% (копия датаграммы в quic-go `datagramQueue`, `buf.As`).
