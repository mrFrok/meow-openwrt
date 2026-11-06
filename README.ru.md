# meow-openwrt

Ядро прокси, совместимое с mihomo, для роутеров на OpenWrt. Написано на Rust и
укладывается в стоковые 16 МБ флеша. Читает тот же `config.yaml` и отдаёт тот же
REST API, что и mihomo, поэтому clashwrt, OpenClash, nikki и обычные
веб-панели работают с ним без изменений.

meow-openwrt — неофициальный форк [meow-rs](https://github.com/meow-rs/meow-rs).
Проект ранний: сборки каждый день работают на одном aarch64-роутере, MIPS
проверяется под qemu. Отчёты с другого железа очень помогают.

[English version](README.md)

## Профили

Один пакет на профиль. Пакеты конфликтуют между собой, ставится один.

| Пакет | Что внутри | aarch64 | armv7 | mipsel | mips |
|---|---|---|---|---|---|
| `meow-tiny` | Trojan, VLESS (Vision, Reality), Hysteria2, mux, DoH/DoT, входы mixed, TProxy и TUN | 6,8 МБ | 7,5 МБ | 9,0 МБ | 8,5 МБ |
| `meow` | tiny плюс Shadowsocks, AnyTLS, VMess, ShadowTLS | 7,2 МБ | 8,0 МБ | 9,7 МБ | 9,1 МБ |
| `meow-full` | всё, что есть в meow-rs, вместе с logbook и traffic ledger | 10,2 МБ | 11,7 МБ | 14,8 МБ | 14,2 МБ |

Размеры даны для статического бинарника после strip. На флеше он занимает
меньше: squashfs хранит `meow-tiny` примерно в 2,6 МБ, JFFS2 в 4,4 МБ, UBIFS
с zlib примерно в 5,5 МБ.

В `meow-tiny` нет Shadowsocks: так экономится место. Если в подписке смешаны
протоколы, SS-узлы загрузятся недоступными, остальные будут работать.

## Архитектуры

| Арх OpenWrt | Примеры устройств | TLS и QUIC |
|---|---|---|
| `aarch64_cortex-a53`, `aarch64_generic`, `-a72`, `-a76` | MT7981/MT7986/MT7988 (Filogic), RK3328/RK3568/RK3588 | BoringSSL, quiche |
| `arm_cortex-a7_neon-vfpv4`, `arm_cortex-a15_neon-vfpv4` | IPQ40xx, IPQ806x | BoringSSL, quiche |
| `arm_cortex-a9_vfpv3-d16`, `arm_cortex-a9_neon` | Armada 370/XP/38x (mvebu), i.MX6 | BoringSSL, quiche |
| `arm_cortex-a9` (без FPU) | BCM4708/4709 (bcm53xx: R7000, RT-AC68U) | BoringSSL, quiche |
| `mipsel_24kc` | MT7621 | BoringSSL, quiche |
| `mips_24kc` | ath79 (big-endian) | rustls, quinn |

BoringSSL собирается только для little-endian, поэтому на big-endian MIPS
вместо него rustls и quinn. Там нет ECH, restls и JLS, а настройка
TLS-отпечатка игнорируется с предупреждением.

## Установка

Скачайте пакет нужного профиля и архитектуры со страницы
[Releases](../../releases), скопируйте на роутер и поставьте.

```sh
# OpenWrt 25.x (apk)
apk add --allow-untrusted /tmp/meow-tiny_*.apk

# OpenWrt 22.03–24.10 (opkg)
opkg install /tmp/meow-tiny_*.ipk
```

Бинарник полностью статический, поэтому один пакет ставится на любой релиз
OpenWrt своей архитектуры, от 22.03 и новее. `tun.auto-redirect` и
`dns.nftset` требуют nftables (fw4, с 22.03). На прошивках с iptables режимы
TProxy, redirect и TUN всё равно работают.

Пакет ставит один файл, `/usr/bin/meow`. Перед переключением проверьте, что
он читает ваш текущий конфиг:

```sh
meow -d /etc/mihomo -f /etc/mihomo/config.yaml -t
```

### С ClashWrt

[ClashWrt](https://github.com/mrFrok/clashwrt) — интерфейс LuCI для
прозрачного проксирования на OpenWrt от автора этого форка. В нём есть мастер
настройки и четыре режима перехвата, один из которых работает на любом ядре.
ClashWrt показывает, когда трафик перехвачен, но никуда не уходит. Под ним
meow тестируется каждый день.

```sh
wget -qO- https://raw.githubusercontent.com/mrFrok/clashwrt/main/install.sh | sh
```

### Запуск под обвязкой

Обвязки запускают ядро по пути. Укажите в их настройке ядра `/usr/bin/meow`
или попробуйте meow вместо mihomo до перезагрузки через bind mount.
Перезагрузка или `umount` возвращают mihomo.

```sh
/etc/init.d/mihomo stop
mount --bind /usr/bin/meow /usr/bin/mihomo
/etc/init.d/mihomo start
```

`meow -v` отвечает `Meow Meta`. Флаги, которые передают обвязки (`-d`, `-f`,
`-t`, `-v`), meow понимает.

## Что добавляет форк

- **`tun.auto-redirect`**. Один ключ конфига ставит правила перехвата: свою
  таблицу nftables `inet meow`, policy routing, редирект DNS и include для fw4.
  Сузить перехват можно через `route-address-set` и `route-exclude-address-set`.
  Сеты обновляются вместе с rule-set, таблица возвращается, если её кто-то
  удалил, а при выходе meow всё убирает. Подробности в [docs/tun.md](docs/tun.md).
- **Узлы WireGuard и AmneziaWG** (`type: wireguard`, для AmneziaWG — с
  `amnezia-wg-option`) работают на модуле ядра: поставьте `kmod-wireguard`
  или `kmod-amneziawg`, и meow сам создаст и настроит интерфейс при первом
  использовании узла. Шифрует ядро, поэтому это быстро даже на MIPS и почти
  не увеличивает бинарник. `reserved` (нужен некоторым WARP) модуль не
  поддерживает.
- **`dns.nftset`**. meow кладёт адреса доменов, которые идут через прокси, в
  набор nftables, и файрвол перехватывает только этот трафик. 80 000 доменов
  занимают в meow около 1,2 МБ, а списками dnsmasq около 28 МБ. Подробности в
  [docs/nftset.md](docs/nftset.md).
- **Поведение mihomo там, где meow-rs отличался**: `tproxy-port` стал
  настоящим TPROXY-входом с UDP, `RULE-SET` работает внутри `AND`/`OR`/`NOT`,
  у proxy-provider работают health-check, ответы REST API сверены с mihomo
  скриптом [`api-compare.py`](openwrt-fork/bench/api-compare.py).
- **Умолчания для малой памяти**: `max-connections` зависит от объёма RAM (256
  на 64 МБ), rule-set читаются с диска потоком в компактные деревья, в простое
  meow занимает около 5 МБ.
- **Big-endian MIPS** на альтернативном стеке TLS и QUIC, описанном выше.

## Замеры

Banana Pi BPI-R4 (MT7988A, aarch64), статические сборки musl без PGO, замер
2026-11-06. Скорость — одна закачка через meow до сервера mihomo на том же
роутере, поэтому она показывает накладные расходы meow, а не скорость канала.

| Профиль | DIRECT TCP | Hysteria2 | VLESS + Reality | Память в простое, рабочий конфиг | Пик RSS |
|---|---|---|---|---|---|
| tiny | 5690 Мбит/с | 306 Мбит/с | 2020 Мбит/с | 5,1 МБ | 14,1 МБ |
| lightweight | 5450 Мбит/с | 312 Мбит/с | 2030 Мбит/с | 5,1 МБ | 14,0 МБ |
| full | 5660 Мбит/с | 323 Мбит/с | 2000 Мбит/с | 6,0 МБ | 16,2 МБ |

Методика и вся история размеров — в
[openwrt-fork/BENCHMARKS.md](openwrt-fork/BENCHMARKS.md).

## Чего пока нет

- XHTTP работает в режимах `stream-one`, `stream-up` и `packet-up` (данные в
  теле, заголовках или cookie), проверено против Xray скриптом
  [`xhttp-test.sh`](openwrt-fork/bench/xhttp-test.sh). Узлы с
  `download-settings` или `x-padding-obfs-mode` загружаются недоступными.
- Ключи конфига, которых нет в meow, например `tun.auto-detect-interface`,
  попадают в лог предупреждением и игнорируются.
- Своего GUI нет. Используйте обвязку или панель на `external-controller`.

## Сборка

Пакет для OpenWrt SDK лежит в [openwrt/package/meow](openwrt/package/meow).
Сделайте ссылку на него в `package/` вашего SDK и соберите вариант:

```sh
ln -s "$PWD/openwrt/package/meow" ~/openwrt-sdk/package/meow
make -C ~/openwrt-sdk package/meow/compile V=s
```

Workflow релизов собирает без Rust из SDK, через скрипты из
[openwrt-fork/bench](openwrt-fork/bench) (`pgo-gen.sh`, `mips-be-build.sh`) и
закреплённый rustup-тулчейн. План работ и его статус — в
[openwrt-fork/PLAN.md](openwrt-fork/PLAN.md).

## Помощь с железом

Если у вас роутер не на aarch64, запустите проверку, которая только читает, и
приложите её вывод к issue. Она ничего не меняет и не выводит адреса, MAC и
ключи.

```sh
wget -O /tmp/hwprobe.sh https://raw.githubusercontent.com/mrFrok/meow-openwrt/main/openwrt-fork/bench/hwprobe.sh
sh /tmp/hwprobe.sh > /tmp/probe.txt
```

## Лицензия

Изменения форка распространяются под GPL-3.0. Код, унаследованный от meow-rs,
по-прежнему доступен под MIT. См. [NOTICE-FORK.md](NOTICE-FORK.md). README
upstream со справочником протоколов и конфига сохранён в
[docs/meow-rs-README.md](docs/meow-rs-README.md).

Форк не связан с авторами meow-rs и не одобрен ими.
