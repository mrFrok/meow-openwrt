## Скорость

- **Hysteria2: несколько QUIC-соединений к узлу.** Одно соединение quiche упирается в одно ядро (~530 Мбит/с на BPI-R4). Теперь потоки и UDP-сессии раскладываются по нескольким соединениям, у каждого свой драйвер. BPI-R4, 4 потока: скачивание 524–539 → 1000–1040 Мбит/с, отдача 482–499 → 842–870 (mihomo в том же замере 691–705 / 676–680). Один поток — без изменений.
  - Новый параметр узла `connections: 1–8` (расширение meow). Не задан — по числу ядер, не больше 4; на одноядерных роутерах одно соединение.
  - Объявленная скорость `down` (Brutal) и бюджет окна приёма (1/16 RAM) делятся между соединениями: сервер в сумме шлёт не больше, памяти уходит не больше, чем раньше.
  - Если сервер или панель ограничивают число соединений с одного IP, поставьте `connections: 1`.

## Speed

- **Hysteria2: parallel QUIC connections per node.** One quiche connection tops out at one core; streams and UDP sessions are now spread over several, each with its own driver. BPI-R4, four streams: download 524–539 → 1000–1040 Mbit/s, upload 482–499 → 842–870 (mihomo 691–705 / 676–680). A single stream is unchanged.
  - New node option `connections: 1–8` (meow extension); unset = one per CPU, at most 4.
  - The declared `down` (Brutal) and the receive-window budget (RAM/16) are split between the connections.
  - Set `connections: 1` if the server limits connections per IP.
