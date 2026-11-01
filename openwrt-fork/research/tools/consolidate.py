"""Merge research/agents/*.json + research/consolidated/opus_review.json into the four consolidated files.
Usage (repo root): python3 -I research/tools/consolidate.py"""
import json, glob, os
A = "research/agents"; C = "research/consolidated"
rev = json.load(open(f"{C}/opus_review.json"))
V = rev.get("verdicts", {})          # "agent:F001" -> {"verdict": confirmed|corrected|downgraded|rejected|spot-checked-ok, "status": ..., "note": ...}
reports = {os.path.basename(p)[:-5]: json.load(open(p)) for p in sorted(glob.glob(f"{A}/*.json"))}
findings, opps, questions = [], [], []
for name, r in reports.items():
    for f in r["findings"]:
        key = f"{name}:{f['id']}"; v = V.get(key, {})
        g = dict(f, id=key, agent=name, verified_by_opus=bool(v), opus_verdict=v.get("verdict", "not-checked"), opus_note=v.get("note", ""))
        if "status" in v: g["status_agent"] = f["status"]; g["status"] = v["status"]
        if "confidence" in v: g["confidence_agent"] = f["confidence"]; g["confidence"] = v["confidence"]
        findings.append(g)
    for o in r["opportunities"]:
        opps.append(dict(o, agent=name))
    for q in r["open_questions"]:
        questions.append({"agent": name, "question": q})
pr = {"critical": 0, "high": 1, "medium": 2, "low": 3}
opps.sort(key=lambda o: pr.get(o["priority"], 9))
meta = {"project": "Mihomo Lightweight", "date": rev["date"], "base": "upstream mihomo v1.19.33 (3f27ef39)"}
json.dump(dict(meta, findings_total=len(findings), verified=sum(f["verified_by_opus"] for f in findings), findings=findings),
          open(f"{C}/technical_findings.json", "w"), ensure_ascii=False, indent=1)
json.dump(dict(meta, decisions_proposed=rev["recommendations"], agent_opportunities=opps),
          open(f"{C}/architecture_recommendations.json", "w"), ensure_ascii=False, indent=1)
json.dump(dict(meta, from_opus=rev["unresolved"], from_agents=questions, hardware_data_needed=rev["hardware_data_needed"]),
          open(f"{C}/unresolved_questions.json", "w"), ensure_ascii=False, indent=1)
json.dump(dict(meta, summary=rev["summary"], measurements=rev["measurements"], contradictions=rev["contradictions"],
               agents=[{"agent": n, "findings": len(r["findings"]), "opportunities": len(r["opportunities"]), "summary": r["summary"], "conclusion": r["conclusion"]} for n, r in reports.items()]),
          open(f"{C}/research_summary.json", "w"), ensure_ascii=False, indent=1)
print(f"{len(reports)} reports, {len(findings)} findings ({sum(f['verified_by_opus'] for f in findings)} verified), {len(opps)} opportunities, {len(questions)} agent questions")
