## Совместимость с конфигами OpenClash и nikki

- `default.yaml` из OpenClash и типовой конфиг nikki теперь проходят `meow -t`:
  - условный `SUB-RULE,(условие),блок`;
  - `proxy-providers` с `type: inline` и `payload:`;
  - `listeners:` с `type: socks`; серверные типы (vmess, trojan, hysteria2, tun и т. п.) пропускаются с предупреждением, а не роняют загрузку.
- Узел из `proxies:`, который не удалось разобрать, остаётся заглушкой с тем же именем: группы и правила, ссылающиеся на него, больше не ломают конфиг.
- **`proxy:` у listener-а**, как в mihomo: соединения этого listener-а идут сразу в указанный прокси или группу, мимо `mode` и правил. Несуществующее имя — ошибка при загрузке (при наличии провайдеров — предупреждение).

## DNS

- Ошибка DoH показывает HTTP-статус: `doh: status 403 (HTTP/1.1 403 Forbidden)` вместо «non-200 status».
- `https://…#ИМЯ` и `tls://…#ИМЯ`: фрагмент без точки — это прокси для запросов, как в mihomo; с точкой — SNI.

## Скорость

- **Hysteria2:** одна копия на принятый кусок и меньше пробуждений драйвера; скачивание через узел на BPI-R4 ~484 → ~540 Мбит/с.
- **PGO** в сборках lightweight и full для aarch64: hy2 TCP +2–4 % на скачивание и +3–15 % на отдачу, CPU на мегабит −5–10 %, REALITY и DIRECT +1–2 %; бинарник +0,4–0,46 МБ. tiny без изменений.

## OpenClash / nikki config compatibility

- OpenClash's `default.yaml` and a typical nikki config pass `meow -t`: conditional `SUB-RULE,(cond),block`, `type: inline` proxy providers, `type: socks` listeners; server-only listener types are skipped with a warning.
- A `proxies:` node that fails to parse stays as a dead placeholder under its name, so groups and rules naming it still load.
- **Listener `proxy:`** (mihomo): the listener's connections go straight to that proxy or group, past `mode` and the rules; an unknown name fails the load.

## DNS

- DoH errors carry the HTTP status. A dotless `#NAME` on `https://` / `tls://` is the query proxy, as in mihomo; a dotted one stays the SNI.

## Speed

- **Hysteria2:** one copy per received chunk, fewer driver wake-ups; download through a node ~484 → ~540 Mbit/s on the BPI-R4.
- **PGO** for aarch64 lightweight and full: hy2 TCP +2–4 % down / +3–15 % up, REALITY and DIRECT +1–2 %, +0.4–0.46 MB. tiny unchanged.
