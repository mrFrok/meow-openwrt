"""Validate agent reports against the shared schema (AGENT_BRIEF.md). Usage: python3 -I validate.py file.json..."""
import json, sys
TOP = ["agent","model","effort","research_date","scope","summary","findings","opportunities","limitations","risks","recommendations","open_questions","conclusion"]
F = {"id":str,"topic":str,"description":str,"technical_details":str,"source_type":str,"sources":list,"confidence":str,"status":str}
O = {"name":str,"description":str,"expected_benefit":str,"implementation_complexity":str,"risks":list,"priority":str}
EN = {"source_type":{"official_docs","source_code","benchmark","experiment","secondary_source"},
      "confidence":{"high","medium","low"},"status":{"confirmed","probable","hypothesis","unverified"},
      "implementation_complexity":{"low","medium","high"},"priority":{"critical","high","medium","low"}}
bad = 0
for path in sys.argv[1:]:
    errs = []
    try:
        d = json.load(open(path))
    except Exception as e:
        print(f"FAIL {path}: {e}"); bad += 1; continue
    errs += [f"missing top key {k}" for k in TOP if k not in d]
    for kind, spec, items in (("finding", F, d.get("findings", [])), ("opportunity", O, d.get("opportunities", []))):
        for i, it in enumerate(items):
            for k, t in spec.items():
                if k not in it: errs.append(f"{kind}[{i}] missing {k}")
                elif not isinstance(it[k], t): errs.append(f"{kind}[{i}].{k} not {t.__name__}")
                elif k in EN and it[k] not in EN[k]: errs.append(f"{kind}[{i}].{k}={it[k]!r} not in enum")
            if kind == "finding":
                for s in it.get("sources", []):
                    if not isinstance(s, dict) or not {"url","file_path","line_reference"} <= set(s): errs.append(f"finding[{i}] bad source {s!r}"[:120]); break
    ids = [f.get("id") for f in d.get("findings", [])]
    if len(ids) != len(set(ids)): errs.append("duplicate finding ids")
    print(("OK  " if not errs else "FAIL") + f" {path}: {len(d.get('findings',[]))} findings, {len(d.get('opportunities',[]))} opportunities" + "".join("\n     - " + e for e in errs[:8]))
    bad += bool(errs)
sys.exit(1 if bad else 0)
