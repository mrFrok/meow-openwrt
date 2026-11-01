# TODO — Mihomo Lightweight

Этап I завершён, D1–D9 приняты 2026-11-01. Идёт этап II.

У каждой оптимизации заполняются поля §17: проблема · ссылка на код · обоснование · ожидаемый эффект · **фактический результат** · риски · тесты · откат (откат = убрать тег или файл).

## Этап II — baseline и профиль сборки
- [x] `bench/livemem.sh` с боевым конфигом на BPI-R4 — сделано 2026-11-01, результаты в BENCHMARKS.md («D4, эксперимент»)
- [x] (2026-11-01, BENCHMARKS.md) livemem upstream vs Lightweight: без env 37.1 → 15.8 МБ anon; авто 48 МиБ+GOGC=50 ≈ 32 МиБ+GOGC=50. Повторить вариант 64MiB (26.9 против 18.0 в прошлый раз). Для clashwrt: ставить GOGC=50 всегда (upstream при 64 МиБ без GOGC держит 37 МБ)
- [ ] ~~Сначала (вместо baseline):~~ `bench/livemem.sh` на BPI-R4 — upstream против Lightweight (arm64, все теги + lw_free_after_load + lw_auto_memlimit), без VPS. Первая проверка всего сделанного по памяти на роутере
- [ ] **tiny на роутерах с ПЗУ 32 МБ** (~16 МБ свободно): ImageBuilder пользователю не подходит; clashwrt уже хранит .xz и распаковывает в /tmp при старте → **размер бинарника = RAM в tmpfs (не выгружается)**. Для Ultra каждый МБ размера = МБ RAM; собрать Ultra под mipsle/mips, замерить размер и xz
- [x] (2026-11-01, BENCHMARKS.md; повторить tcp-proxy-dl4 и udp-proxy-dl) Baseline upstream v1.19.33 на BPI-R4: `bench/baseline.sh` по сценариям BENCHMARKS.md. **Причина обхода найдена (2026-11-01, по ssh):** (1) `bypass_set_mode=auto` разрешается в `include` (`fw.sh:478`: `ip daddr != @bypass4 ip daddr != @bypass4_dns return`) — в mihomo идут только адреса из списков блокировок, VPS туда не входит; (2) при `off` под iperf3 срабатывает watchdog (`watchdog_action=direct`, timeout 5 с): в logread `auto-bypass ON: proxy path is down` → таблица сброшена, трафик снова напрямую. Рецепт: `uci set clashwrt.config.bypass_set_mode=off; uci set clashwrt.config.watchdog=0; uci commit clashwrt; /usr/libexec/clashwrt/fw.sh apply` (+ перезапуск watchdog, если он не читает UCI на лету), после замера вернуть `auto` и `1`. Менять конфиг роутера — только по слову пользователя. **(3) 2026-11-01, прогон `bench/baseline-run.sh`:** на ПК-источнике (этой машине) включён Throne в TUN-режиме (`throne-tun`, таблица 2022) — он сам туннелирует трафик на VPS (видно как соединения ПК→VPS:4443), до mihomo на роутере iperf3 не доходит. Пользователь добавит в Throne правило 217.60.33.169 → direct; после этого перезапустить `bench/baseline-run.sh upstream`. Настройка и возврат роутера скриптом проверены (reload/pin/restore — 204)
- [x] Outbound-теги (2026-11-01, `lightweight/gen_outbound_stubs.py`): no_hysteria, no_hysteria2, no_tuic, no_shadowquic, no_masque, no_trusttunnel, no_wireguard (только вместе с no_masque no_openvpn no_zerotier), no_ssh, no_openvpn, no_mieru, no_sudoku, no_snell. arm64 со всеми + lean-теги без gvisor: 31 850 620 байт
- [x] Inbound-теги (2026-11-01, `lightweight/gen_inbound_stubs.py`): те же имена тегов отключают и inbound — no_tuic, no_hysteria2 (sing_hysteria2 + hysteria2_realm), no_shadowquic, no_sudoku, no_trusttunnel, no_snell (стаб листового пакета `listener/<pkg>`, upstream-файлы не тронуты), no_mieru (стаб `listener/inbound/mieru.go`). Отключённый inbound в конфиге: ошибка в лог при Listen, ядро работает. go vet ./... с тегами и без, тесты listener/inbound (4321 / 3932). arm64, все outbound+inbound теги + lean без gvisor: **28 704 892** байт (было 31 850 620, −3.15 МБ). quic-go остаётся — держат DNS h3/DoQ, xhttp-h3, sniffer
- [x] no_pprof (2026-11-01, `hub/route/pprof_lw*.go`, в server.go 2 строки): `/debug` (только при log-level debug) отдаёт 404 вместо pprof; уходят chi/middleware, net/http/pprof, google/pprof. arm64 все теги: 28 704 892 → **28 115 068** (−590 КБ)
- [x] no_restls (2026-11-01, `transport/restls/stub.go`): плагин restls для SS/snell/trojan/vless/vmess и inbound restls; уходит restls-client-go. arm64 все теги: 28 115 068 → **27 394 172** (−720 КБ). Тесты с тегом: `-skip Restls` (тесты restls внутри общих *_test.go, upstream не трогаем) — 4704 passed
- [x] no_jls (2026-11-01, `transport/jls/stub.go`): плагин JLS; `jls-tls` уходит только вместе с no_shadowquic (ShadowQUIC тянет jls-quic-go). arm64 все теги: 27 394 172 → **26 869 884** (−524 КБ). Тесты: `-skip "Restls|JLS|Jls"` — 4562 passed
- [x] no_kcptun (2026-11-01, `transport/kcptun/stub.go`, Config продублирован из common.go — при rebase сверять поля): уходят kcp-go и reedsolomon. arm64 все теги: 26 869 884 → **26 214 524** (−655 КБ). Тесты: `-skip "Kcp|KCP|kcp"` — 4786 passed
- [ ] **Дальше:** profiles/*.tags и матрица архитектур (+ CI с SHA-256), затем no_quic (vless xhttp-h3, dns/doh.go h3, dns/doq.go, component/sniffer/quic_sniffer.go; < 1 МБ) — **не делать для tiny**: без quic-go нет Hysteria2 (пользователь, 2026-11-01), а выигрыш < 1 МБ
- [ ] (низкий приоритет) no_geosite_dat: protobuf всё равно остаётся — его использует сам VLESS (`transport/vless/conn.go`, `config.pb.go`: addons/Flow), а в полной сборке с тегами ещё и `listener/sing_vless`. Тег убрал бы только `component/geodata` (~200–400 КБ, оценка) ценой правил GEOSITE. Импортёры geodata: main.go, config/config.go, hub/executor, component/updater/update_geo.go, rules/common/{geosite,geoip,ipasn}.go — правок в горячих файлах много
- [ ] Остальные теги D2: no_quic, no_ssr, no_restls, no_kcptun, no_jls, no_geosite_dat (outbound/inbound-теги и no_pprof уже сделаны). Замер размера каждого: `bench/size.sh` с SETS=<теги>
- [ ] `profiles/{tiny,lightweight,full}.tags` + сборка матрицы архитектур; CI-скрипт с SHA-256. Локальная матрица: `lightweight/build.sh` (2026-11-01) — профили × 14 linux-арх (имена как `output` в upstream build.yml), `mihomo-linux-<arch>-<profile>-<ver>.gz` + `.gz.sha256` + `checksums.txt`, отчёт `sizes.txt` с gz/xz. Осталось: workflow на его основе
- [ ] Переиндексировать Xerj: `xerj autoindex ~/Projects/mihomo-lightweight --prefix mlw-fork --fresh`

### Как гонять baseline (заметки 2026-11-01)
- VPS 217.60.33.169 (Debian/Ubuntu, sudo): iperf3 на 5201–5204 (PROXY) и 5211–5214 (DIRECT); 5201 держит systemd `iperf3.service`. Файрвол VPS закрыт по умолчанию — открыть 5201:5214 tcp+udp на время замера, потом закрыть.
- Правила наверх `rules:` на время замера: `DST-PORT,5211-5214,DIRECT` и `DST-PORT,5201-5204,Proxy` (группа `Proxy`). Плюс закрепить узел, выключить watchdog, если включён.
- WAN роутера за CGNAT (100.107.x), публичный адрес провайдера 217.60.183.177 — по источнику на VPS DIRECT и PROXY не различить, если узел прокси там же.
- Проверка «трафик идёт через ядро»: тики CPU mihomo за 5 с под нагрузкой (`/proc/<pid>/stat` поля 14+15), не `/connections` (jsonfilter печатает JSON с пробелами, разбор хрупкий) и не `top -bn1` (первый кадр BusyBox = 0%).
- ПК: kitty подменяет `ssh` на `kitten ssh`, неинтерактивные команды падают — запускать из bash или после `unalias ssh`.
- Функции `run`/`run500`/`series` для ПК — в переписке 2026-11-01; при следующем запуске оформить в `bench/drive-pc.sh`.

### Дальше ужимать размер (2026-11-01)
- [x] (2026-11-01) DCE методов работает: в `-ldflags=-dumpdep` (tiny, arm64) нет ни одного символа `<ReflectMethod>`; protobuf `descfmt` зовёт `MethodByName` только с константой. Рычага нет. По пакетам (ultra arm64, `go tool nm -size`): своё mihomo ~2.0 МБ, funcdata ~1.6, runtime 0.8, quic-go/utls/protobuf/metacubex-http/mipstack по 0.4–0.5 МБ — крупного одиночного кандидата нет. `crypto/internal/fips140/drbg.memory` 32 МиБ — BSS для FIPS-энтропии, не в файле и не в RSS
- [ ] **Легаси и серверный код** (запрос пользователя 2026-11-01), отдельными тегами:
  - легаси-конфиг (только под тегом и только в tiny/lightweight; без тега конфиг стандартного mihomo должен работать как есть): верхнеуровневые `tuic-server:`, `ss-config`/`vmess-config`, `tunnels` (в `listener/listener.go` + `hub/executor`), устаревшие опции и миграции старого формата конфига, Hysteria v1, SSR (`no_ssr`);
  - серверная сторона: все inbound, кроме нужных роутеру (tun, redir, tproxy, mixed/socks/http, dns); серверные части транспортов (reality/shadowtls/sudoku/vless/trojan server, restls/jls server уже под тегами). **REST API не трогаем** — он должен совпадать со стандартным mihomo (ядро для всех, не только для clashwrt)
- [x] **Состав протоколов tiny/lightweight (решение пользователя 2026-11-01, сделано: no_vmess, no_ssr, no_anytls, no_gost_relay, no_server_inbound; профили обновлены, размеры в profiles/README.md):** VLESS, Trojan, Shadowsocks, Hysteria2 + SOCKS5/HTTP; AmneziaWG/WireGuard — модулем ядра Linux (`interface-name` в прокси `type: direct`), userspace WG (+917 КБ arm64) только в full. Сделано: `no_vmess` (−393 КБ), пропуск узлов отключённых протоколов в провайдерах (`outbound.DisabledError`). Осталось: `no_anytls`, `no_ssr`, серверные inbound (SS/VLESS/Trojan/Hysteria2/AnyTLS; эксперимент: −640 КБ arm64), легаси-ключи `ss-config`/`vmess-config`/`tuic-server` (вместе с серверными −1.09 МБ arm64, −1.28 МБ mipsle; `tunnels` не легаси — оставить), no_wireguard в lightweight
- [x] (2026-11-01, решение пользователя) lightweight = современные протоколы без легаси (вырезаны VMess, SSR, Hysteria v1, Snell, gost-relay, kcptun, OpenVPN, SSH); MASQUE тянет userspace WireGuard — он остаётся в lightweight. `no_server_inbound` во всех профилях, включая full
- [x] Текст ошибки TUN без драйвера (2026-11-01, `listener/sing_tun/tun_hint_lw_*.go`): «kernel TUN driver is not available (/dev/net/tun); load the tun module (OpenWrt: install kmod-tun)»; проверено в user namespace со скрытым /dev/net
- [ ] Теги для того, что роутеру-клиенту не нужно: inbound-серверы VLESS/VMess/Trojan/SS, поиск процесса (PROCESS-NAME), лишние снифферы, DNS-серверы DoH/DoQ, транспорты gRPC/xhttp — оценка 1–3 МБ
- [ ] (только tiny, после замера CPU) `-gcflags=-l` — −5–10% размера ценой CPU
- [ ] CI: матрица профилей × архитектур + ручная сборка с выбранными тегами (`workflow_dispatch`)
- Не делаем: TinyGo (reflect/protobuf, go:linkname, ассемблер, GC и планировщик), плагины Go, протоколы отдельными процессами

## Этап III — память (D4)
- [x] `lw_free_after_load` (debug.FreeOSMemory после ApplyConfig): anon 30.4 → 18.0 МБ на BPI-R4 — включить в профили tiny и lightweight
- [x] ~~Параллелизм загрузки провайдеров~~ — не помогает (41.9 МБ), тег удалён
- [x] Такой же возврат памяти после обновления провайдеров (2026-11-01, `component/resource/fetcher_lw.go`, тот же тег `lw_free_after_load`): хук после `onUpdate` в `Fetcher.loadBuf` — фоновые, API- и file-watch-обновления rule- и proxy-провайдеров; один `FreeOSMemory` через 2 с после последнего обновления в серии. Проверено: go vet с тегом и без, сборка. **Не замерено:** на BPI-R4 RssAnon до/после `curl -X PUT -H "Authorization: Bearer $S" http://$C/providers/rules/<имя>` (самый большой .mrs), сборка с тегом против без тега
- [ ] Уменьшить сам пик: какой провайдер его даёт (profile heap при старте), .mrs без binary.Read
- [ ] (старое) Параллелизм загрузки провайдеров по профилю (файл с тегом рядом с `hub/executor/concurrent_load_*.go`); замер всплеска gctrace на BPI-R4
- [ ] .mrs reader без `binary.Read` на каждый элемент (rules/provider, component/trie)
- [ ] Размер трубы splice (sing `splice_linux.go:12`) — через replace на форк sing или патч; замер памяти ядра (Slab, PageTables) при 100/500 потоках
- [x] GOMEMLIMIT/GOGC по /proc/meminfo, если env пуст (2026-11-01, тег `lw_auto_memlimit`, `lightweight/memlimit` + `memlimit_lw.go`): лимит = MemTotal/8 в пределах [24, 48] МиБ, GOGC=50. Формула выбрана без пользователя — **подтвердить** по результатам livemem. clashwrt (авто): 48 МиБ при RAM < 256 МБ, 96 МиБ при < 512 МБ, выше — без предела (64 МиБ на BPI-R4 был тестовым). Расхождение: при RAM ≥ 512 МБ clashwrt, видимо, не передаёт env → lw_auto_memlimit поставит 48 МиБ вопреки «без предела»; лечится `GOMEMLIMIT=off` из clashwrt (1 строка). Если замер покажет, что 32 МиБ заметно лучше 48 — пересмотреть пороги и в clashwrt. Тест расчёта есть; на роутере не проверено (clashwrt и так ставит env, проверять запуском без env)
- [ ] UDP > 8 КиБ при `with_low_memory` — проверить, нет ли обрезания

- [ ] quic-go под `with_low_memory`: уменьшить окна приёма в `quic.Config` (по умолчанию до 6 МБ на поток / 15 МБ на соединение), без форка quic-go; замер RSS при 100 потоках через Hysteria2 на BPI-R4
- [x] Доля quic-go в бинарнике (2026-11-01, `go tool nm -size`, arm64 все теги): quic-go ~384 КБ символов, http3+qpack ~80 КБ (с pclntab примерно ×2). **`no_http3` не делаем** — не окупается; `no_quic` для Ultra даст < 1 МБ, приоритет ниже no_geosite_dat

- [ ] Залп одновременных dial к одному узлу Reality (VPS sportzal:443) падает по таймауту у upstream: 1 запрос `/proxies/Proxy/delay` — 61 мс, 4–8 параллельно — все Timeout; iperf3 `-P 4` через прокси и UDP-тест через Hysteria2 (его TCP-управление идёт через Reality) не сняты. fail2ban/ufw на VPS не виноваты. Повторить на нашем ядре: та же картина → среда/сервер, иначе — разбирать.

## Этап IV — CPU
- [ ] Следующая сессия (порядок, 2026-11-01): (1) ~~Hysteria2-клиент: пакетная запись в TUN~~ — сделано (BENCHMARKS.md «Hysteria2-клиент — пакетная запись в TUN», 1.48 → 1.09 %/Мбит, −26%); с сервером на ноутбуке 1.55 → 0.66 %/Мбит, 138 → 240 Мбит/с; (1б) **отдача через Hysteria2**: 129 Мбит/с при 35% потерь внутри mihomo (BENCHMARKS «Сервер на ноутбуке») — пакетная отправка датаграмм (TUN batch read → quic-go SendDatagram по одной); тот же путь ограничивает и сервер стенда; (2) IPv6 через TUN на роутере; (3) сервер Hysteria2 быстрее для стенда (не mihomo-inbound через veth, или выяснить, почему он упирается в ~100 Мбит/с при 0.74 ядра); (4) `-gcflags all=-l std=` и MIPS — когда будет MT7621. Сделано: Reality-профиль, `-gcflags all=-l` на TCP
- [ ] (отложено) Шифрование в ядре. BPI-R4 (ARMv8 AES/PMULL, нет kmod-tls/safexcel): Go уже на аппаратном AES — выигрыш kTLS только на копированиях/syscalls, решать по профилю (1). MT7621 (MIPS 880 МГц, без AES/FPU, EIP-93 — без GCM/ChaCha, проверить): в ядре есть MIPS-asm ChaCha20/Poly1305 — kTLS TX + splice для VLESS+Reality имеет смысл только с ChaCha (отпечаток uTLS ставит AES-GCM первым — проверить вариант с ChaCha первым). Начать с бенча Go vs ядро (kTLS loopback/AF_ALG) на mipsle-softfloat; нужен роутер MT7621 у пользователя
- [x] (2026-11-01, BENCHMARKS.md «Этап IV: TCP через Reality») CPU-профиль TCP через Reality без VPS: сервер в netns (`bench/reality-server.yaml`). 940 Мбит/с: ↓4 — 33% ядра, ↑ — 69% (DIRECT ↑ — 52%); надбавка — почти только AES-GCM (аппаратный), остальное syscalls. kTLS на BPI-R4 не нужен
- [ ] Цена `-gcflags` для tiny: `all=-l` на TCP Reality замерен — ↑ +4% CPU (больше шума), в tiny по умолчанию не включаем. Осталось: `all=-l std=` (−1.44 МБ) и UDP Hysteria2; MIPS (нужен роутер MT7621) — там размер важнее, решать по нему
- [ ] quic-go: проверить, что UDP GSO и пакетный приём реально включаются на ядре 6.12 BPI-R4; профиль CPU под Hysteria2 (сборка без no_pprof). Форк quic-go — только если профиль укажет на сам quic-go, а не на крипто/syscalls
- [x] (2026-11-01, BENCHMARKS.md «recvmmsg + UDP GRO») UDP через TUN, DIRECT: recvmmsg + пакетная запись в TUN со склейкой GRO (форк sing-tun `third_party/sing-tun`, `stack_system_batch_lw.go`) — на насыщении ~303 → ~738 Мбит/с на BPI-R4. Осталось: прокси-путь (Hysteria2/quic-go отдаёт датаграммы по одной — батч на стороне WriteBack не включается); CPU на малой нагрузке (300M: 90 → 80%) — UDP_GRO на сокете?; IPv6 через TUN не проверен на роутере
- [ ] Сравнить TUN system / mips / tproxy / redirect на BPI-R4 (CPU на Гбит/с)
- [ ] `tun.gso` на 6.12; ChaCha20 vs AES-GCM на arm64 и (через добровольцев) на mt7621
- [ ] Go 1.27 vs 1.26 (RSS, CPU)

## Этап V — быстрый путь DIRECT (D5)
- [ ] Модуль `bypass`: статический анализ правил («IP-определимость»), тесты на таблице правил с AI/RU/CDN-кейсами
- [ ] Поток вердиктов (ядро → обвязка): формат и транспорт (unix-сокет или API), timeout, ct mark
- [ ] Обвязка в clashwrt: набор `learned`, правило до перехвата, flush + `conntrack -D --mark` при reload/start/stop
- [ ] Тесты безопасности (BENCHMARKS.md → «Безопасность») — до любых замеров скорости

## Этап VI — эксперименты
- [ ] sockmap vs splice (D6) на BPI-R4, только если этап V готов
- [ ] PGO с профилем с MIPS/ARMv7 (добровольцы)

## Ждём данные
- [x] `hwprobe.sh` с BPI-R4 (в BENCHMARKS.md)
- [ ] `hwprobe.sh` с роутеров добровольцев 4pda (mt7621, ath79, ipq40xx, filogic) — ожидается 2026-11-02
- [x] (2026-11-01) 5 устройств от одного добровольца: Cudy WBR3000UAX, Redmi AX6000 (filogic), NanoPi R3S/R76S/R6S (rockchip) — все aarch64 с AES/PMULL, ≥ 485 МБ RAM, ≥ 80 МБ свободного ПЗУ; сводка в BENCHMARKS.md. MIPS по-прежнему нет — mt7621/ath79 ждём
- [x] (2026-11-01) Второй пакет: Mi 4A Gigabit v2 (MT7621, 42 МБ свободной RAM, нет kmod-tun), Cudy WBR3000UAX #2, Keenetic Extra II (MT7628), WR841N v8 (ath79) — сводка в BENCHMARKS.md. tiny mipsle = 27.3 МБ: на MT7621 ядро держать на jffs2, а не в tmpfs; 64-МБ устройства не тянут. Ждём ещё один пакет
- [ ] **tiny под MIPS**: `-gcflags=all=-l` даёт 27.3 → 24.6 МБ — решить после замера CPU на MIPS (скрипт для добровольца: AES/ChaCha, TCP/UDP-копирование, сборки с -l и без). На стоковом MT7621 (16 МБ ПЗУ) ядро всё равно в tmpfs: проверить, уходят ли страницы бинарника из tmpfs в zram-swap (OpenWrt `zram-swap`) — тогда холодный код стоит ~1/3 RAM; для clashwrt — режим «распакованный бинарник на overlay, если места хватает»
- [ ] Минимальные требования в README (≈128 МБ RAM; MIPS — только ядро на ПЗУ) и вариант clashwrt «распакованный бинарник на overlay, если места хватает»
- [x] Текст ошибки при `tun: enable: true` без `/dev/net/tun` — сделано, см. этап II

## Для clashwrt (заметки, делаются в сессиях clashwrt)
- Выбирать профиль ядра по `proxies:` подписки: нет hy2/tuic → tiny, иначе lightweight
- WireGuard/AmneziaWG через kmod вместо userspace (вариант B)
- GOMEMLIMIT: при RAM ≥ 512 МБ передавать `GOMEMLIMIT=off`, если «без предела» должно оставаться без предела с ядром lw_auto_memlimit; ветка `mem-gogc` (ручной предел + GOGC=50) ждёт решения
- Watchdog под нагрузкой (iperf3) ложно включает auto-bypass (проба с timeout 5 с не проходит) — для clashwrt: не считать путь упавшим, пока через ядро идёт трафик, или увеличить timeout под нагрузкой

## Прочее
- [x] Тестовые бинарники `out/d4/*` (4×56 МБ) убраны из истории (filter-branch, 2026-11-01), `/out/` в .gitignore. Старая история — в ветке `backup/pre-strip-out`; удалить её и сделать `git gc --prune=now` — по слову пользователя, до первого push
- [x] Удалены устаревшие индексы Xerj `mihomo-docs` и `mihomo-test-config` (с согласия пользователя)
- [ ] Проверить в clashwrt `bypass4_dns`: тот же риск общих IP CDN, что и у доменного динамического обхода
