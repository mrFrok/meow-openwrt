## Что нового

- **XHTTP: режимы `stream-up` и `packet-up`**, как в Xray. Данные можно передавать в теле, заголовках или cookie, session и seq класть в path, query, header или cookie. Работает и `uplink-http-method: GET`. `mode: auto` выбирает режим как Xray: `stream-one` с REALITY, иначе `packet-up`. Проверено против Xray 26 и на живых узлах с REALITY. `download-settings` и `x-padding-obfs-mode` пока не поддерживаются.
- **DoH по HTTP/2.** Раньше DoH-клиент говорил только по HTTP/1.1. Серверы, которые его не принимают, и Quad9 (`dns11.quad9.net`) не отвечали, и запросы молча уходили на обычный `nameserver`. Теперь клиент договаривается о h2, и все запросы идут через одно соединение.
- **Hysteria2 быстрее.** Добавлены UDP GSO/GRO, quiche собирается с `-O3`. На BPI-R4 TCP через узел: скачивание 315–328 → 496–513 Мбит/с, CPU на мегабит 0,43 → 0,23.
- **Новые пакеты для Cortex-A9:**
  - `arm_cortex-a9_vfpv3-d16` и `arm_cortex-a9_neon`: mvebu (Armada 370/XP/38x), i.MX6;
  - `arm_cortex-a9` без FPU: bcm53xx (R7000, RT-AC68U).
  
  Сборка для A7 теперь подходит и для `arm_cortex-a15_neon-vfpv4` (ipq806x). Новые сборки проверены под qemu, отчёты с реального железа приветствуются.
- **MIPS (big-endian):** TLS на rustls предлагает ChaCha20 первым, на CPU без AES это в разы быстрее.
- **CI:** SDK качается с зеркала и кэшируется по своему sha256. Сборка релиза занимает около 7 минут вместо 45.

## What's new

- **XHTTP `stream-up` and `packet-up`**, Xray-compatible: data in the body, headers or cookies, session and seq in path, query, header or cookie, and `uplink-http-method: GET`. `mode: auto` resolves like Xray. Checked against Xray 26 and on live REALITY nodes.
- **DoH over HTTP/2.** Servers that refuse HTTP/1.1, and Quad9, used to fail and fall through silently to the plain nameserver.
- **Hysteria2:** UDP GSO/GRO and quiche at `-O3`. TCP through a node on the BPI-R4 went from 315–328 to 496–513 Mbit/s, CPU per Mbit from 0.43 to 0.23.
- **Cortex-A9 packages** (`arm_cortex-a9_vfpv3-d16`, `arm_cortex-a9_neon`, soft-float `arm_cortex-a9`), plus an `arm_cortex-a15_neon-vfpv4` label on the A7 build.
- **Big-endian MIPS:** rustls offers ChaCha20 first.
