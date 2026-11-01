# Research agent brief — Mihomo Lightweight (shared by all agents)

Project: a fork of mihomo (Clash.Meta) tuned for OpenWrt routers with little RAM/CPU.
Targets (goals, not promises): binary <=16 MB, RSS idle <=10 MB, <=20 MB in normal use, minimal CPU,
TCP/UDP throughput near the hardware limit. Priorities: stability > RAM > CPU > throughput > latency > size > features.
Three build profiles will be designed: Ultra Lightweight, Lightweight (router), Performance.
Key open question for the whole project: can DIRECT connections, once classified by mihomo, get a SAFE
kernel fast path (nftables sets / flowtable / eBPF) without ever letting PROXY traffic bypass the proxy
(no leaks, no DNS leaks, correct IPv6, correct invalidation on config change, auto-disable when policy cannot be guaranteed).

## Where things are (all read-only for you)
- Fork source (upstream mihomo v1.19.33, commit 3f27ef39): ~/Projects/mihomo-lightweight
- Go module sources: `cd ~/Projects/mihomo-lightweight && go list -m -f '{{.Dir}}' <module>`; key ones linked in ~/Projects/mihomo-lightweight-refs/deps/
- Go 1.27.3 stdlib/runtime: /usr/lib/go/src (upstream CI builds with Go 1.26)
- Linux v6.12.112 (= kernel of OpenWrt 25.12.6 stable) sparse subset: ~/Projects/mihomo-lightweight-refs/linux
- OpenWrt v25.12.6 sparse (target/linux/generic config-6.12, package/kernel/linux modules, mediatek/ramips/ath79/ipq40xx/qualcommax/rockchip, firewall4 pkg, dnsmasq): ~/Projects/mihomo-lightweight-refs/openwrt
- firewall4, cilium/ebpf, sing-box (latest): ~/Projects/mihomo-lightweight-refs/{firewall4,cilium-ebpf,sing-box}
- Related project (consumer of the core, OpenWrt LuCI app with its own nft bypass set + flow offload switch): ~/Projects/clashwrt (see clashwrt/files/usr/libexec/clashwrt/setctl.sh, fw.sh)

## Xerj (local code search, use it)
MCP tool `xerj_search` with `prefix` and a plain-string `query` does definition-first code search and returns path + match_line.
Prefixes: mlw-fork, mlw-deps, mlw-goruntime, mlw-kernel, mlw-openwrt, mlw-fw4, mlw-ebpf, mlw-singbox. Pass `max_tokens` (e.g. 600) to keep answers small.
(If the MCP tool is not loaded, fetch it with ToolSearch "select:mcp__xerj__xerj_search".) Grep/Read are fine too.
The Xerj Corpus Hub has NO corpora for these topics — do not spend calls on it.

## Rules
1. Study your area; prefer primary sources: actual source code (cite file:line), official docs (kernel.org, go.dev, openwrt.org, ebpf.io). Check versions are current (today is 2026-11-01).
2. Separate confirmed facts from hypotheses. Never invent benchmark numbers. Anything you could not confirm is `hypothesis` or `unverified`.
3. Do not make final architecture decisions — give options, trade-offs, recommendations.
4. Do not modify the fork, refs or any repo. You may write ONLY your own report file research/agents/<name>.json.
   Small experiments (e.g. a Go microbenchmark, a build to measure something) go in your own dir under
   /tmp/claude-1000/-home-mrfrok-Projects-clashwrt/0712caf5-593b-4903-aa33-590a1010ee6c/scratchpad/<agent-name>/ and are reported with source_type "experiment".
   Do not run anything that changes the host network/firewall.
5. Budget: about 40 tool calls. At most 25 findings and 10 opportunities. Be dense, no filler.
6. Validate your file: `python3 -I -m json.tool research/agents/<name>.json >/dev/null`.
7. Final message to the main agent: at most 15 lines — file path, number of findings, the 5 most important findings (one line each), top open questions.

## Report format (JSON, no comments; all fields required)
{
  "agent": "agent_name",
  "model": "Claude Sonnet 5.5",
  "effort": "Medium",
  "research_date": "2026-11-01",
  "scope": [],
  "summary": "",
  "findings": [
    {
      "id": "F001",
      "topic": "",
      "description": "",
      "technical_details": "",
      "source_type": "official_docs|source_code|benchmark|experiment|secondary_source",
      "sources": [ { "url": "", "file_path": "", "line_reference": "" } ],
      "confidence": "high|medium|low",
      "status": "confirmed|probable|hypothesis|unverified"
    }
  ],
  "opportunities": [
    {
      "name": "",
      "description": "",
      "expected_benefit": "",
      "implementation_complexity": "low|medium|high",
      "risks": [],
      "priority": "critical|high|medium|low"
    }
  ],
  "limitations": [],
  "risks": [],
  "recommendations": [],
  "open_questions": [],
  "conclusion": ""
}
Use exactly one enum value per field (e.g. "source_code", not "source_code|official_docs"). For file paths use paths relative to the tree they come from, prefixed with the tree name, e.g. "fork:adapter/parser.go", "linux:net/netfilter/nf_flow_table_core.c", "mod:github.com/metacubex/sing-tun@v0.4.28/stack_system.go", "go:runtime/mgc.go", "openwrt:target/linux/generic/config-6.12".
