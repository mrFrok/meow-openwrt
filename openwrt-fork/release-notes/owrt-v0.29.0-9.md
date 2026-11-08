## Исправление

- **Провайдеры за симлинками снова загружаются.** В -8 из апстрима пришла проверка, которая отказывалась читать `rule-providers` / `proxy-providers`, если их папка — симлинк наружу (например `/etc/mihomo/geo -> /tmp/…`, как делают clashwrt, OpenClash и nikki). Ядро падало с `escapes the provider directory … via a symlink`. Проверка снова лексическая, как в mihomo: симлинки администратора разрешены, `../` и абсолютные пути вне папки по-прежнему запрещены. `SAFE_PATHS` работает.

## Fix

- **Providers behind symlinked dirs load again.** -8 picked up an upstream check that refused rule/proxy providers whose directory is a symlink out of the config dir (`/etc/mihomo/geo -> /tmp/…`, as clashwrt, OpenClash and nikki set up), failing with `escapes the provider directory … via a symlink`. The check is lexical again, as in mihomo; `../` and absolute paths outside the dir are still refused, `SAFE_PATHS` still works.
