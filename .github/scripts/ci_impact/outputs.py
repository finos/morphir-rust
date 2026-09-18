"""Render a Plan as GitHub Actions outputs or as text for humans."""

from __future__ import annotations

import json

from .classify import Plan


def output_key(job_name: str) -> str:
    return "job_" + job_name.replace("-", "_")


def _flag(value: bool) -> str:
    return "true" if value else "false"


def render_github(plan: Plan) -> str:
    lines = [
        f"all={_flag(plan.all)}",
        f"crates={json.dumps(list(plan.crates))}",
        f"cargo_packages={plan.cargo_packages}",
        f"job_rust={_flag(plan.rust)}",
    ]
    lines.extend(f"{output_key(name)}={_flag(enabled)}" for name, enabled in plan.jobs.items())
    matrix = [{"id": item.short_id, "package": item.package} for item in plan.extensions]
    lines.append(f"extensions={json.dumps(matrix)}")
    lines.append(f"reasons={json.dumps(list(plan.reasons))}")
    return "\n".join(lines) + "\n"


def render_text(plan: Plan) -> str:
    lines = [f"all: {_flag(plan.all)}"]
    lines.append("crates: " + (", ".join(plan.crates) if plan.crates else "(none)"))
    lines.append("cargo packages: " + (plan.cargo_packages or "(workspace)" if plan.all else plan.cargo_packages or "(none)"))
    lines.append(f"rust jobs: {'run' if plan.rust else 'skip'}")
    for name, enabled in plan.jobs.items():
        lines.append(f"{name}: {'run' if enabled else 'skip'}")
    lines.append(
        "extensions: " + (", ".join(item.short_id for item in plan.extensions) if plan.extensions else "(none)")
    )
    for reason in plan.reasons:
        lines.append(f"reason: {reason}")
    return "\n".join(lines) + "\n"
