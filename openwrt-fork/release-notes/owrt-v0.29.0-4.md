## Что нового

- **DNS по TCP.** DNS-сервер meow слушал только UDP. dnsmasq пересылает TCP-запрос в upstream тоже по TCP, поэтому клиенты, которые резолвят по TCP, получали REFUSED на любой некэшированный домен через `server=127.0.0.1#1053`. У них не открывался ни один сайт, а у клиентов на UDP всё работало. Теперь `dns.listen` слушает UDP и TCP на одном адресе, как mihomo. Если TCP-bind не удался, в лог пишется предупреждение, и DNS продолжает работать по UDP.

## What's new

- **DNS over TCP.** The meow DNS server listened on UDP only. dnsmasq relays a TCP query upstream over TCP, so clients that resolve over TCP got REFUSED for every uncached name through `server=127.0.0.1#1053`. `dns.listen` now serves UDP and TCP on the same address, as mihomo does. A failed TCP bind logs a warning and leaves UDP running.
