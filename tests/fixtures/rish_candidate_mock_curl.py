#!/usr/bin/env python3
"""Bounded GitHub REST fixture for the rish candidate resolver test."""

from __future__ import annotations

import json
import os
import sys
from typing import Any


REPOSITORY = "CyberBASSLord-666/termux-mcp-edge"
COMMIT = "a" * 40
OLD_COMMIT = "b" * 40
PULL_REQUEST = 328


def emit(value: Any) -> None:
    json.dump(value, sys.stdout, separators=(",", ":"))
    sys.stdout.write("\n")


def review(
    review_id: int,
    login: str,
    state: str,
    *,
    commit: str | None = COMMIT,
    association: str = "MEMBER",
    submitted_at: str | None = None,
) -> dict[str, Any]:
    if submitted_at is None:
        submitted_at = f"2026-07-30T12:{review_id % 60:02d}:00Z"
    return {
        "id": review_id,
        "user": {"login": login, "type": "User"},
        "author_association": association,
        "state": state,
        "commit_id": commit,
        "submitted_at": submitted_at,
    }


def reviews_for(case: str) -> list[dict[str, Any]]:
    approvals = [
        review(1, "reviewer-one", "APPROVED", association="OWNER"),
        review(2, "reviewer-two", "APPROVED", association="COLLABORATOR"),
    ]
    if case == "insufficient":
        return approvals[:1]
    if case == "outdated":
        return [
            review(1, "reviewer-one", "APPROVED", commit=OLD_COMMIT),
            review(2, "reviewer-two", "APPROVED", commit=OLD_COMMIT),
        ]
    if case == "dismissed":
        return approvals + [
            review(3, "reviewer-one", "DISMISSED", submitted_at="2026-07-30T13:00:00Z")
        ]
    if case == "changes_requested":
        return approvals + [
            review(
                3,
                "reviewer-one",
                "CHANGES_REQUESTED",
                submitted_at="2026-07-30T13:00:00Z",
            )
        ]
    if case == "commented_after_approval":
        return approvals + [
            review(
                3,
                "reviewer-one",
                "COMMENTED",
                submitted_at="2026-07-30T13:00:00Z",
            )
        ]
    if case == "duplicate_reviewer":
        return [
            review(1, "reviewer-one", "APPROVED", association="OWNER"),
            review(2, "reviewer-one", "APPROVED", association="OWNER"),
        ]
    if case == "self_approval":
        return [
            review(1, "candidate-author", "APPROVED", association="OWNER"),
            review(2, "reviewer-one", "APPROVED", association="MEMBER"),
        ]
    if case == "solo_owner_self_approval":
        return [
            review(1, "candidate-author", "APPROVED", association="OWNER"),
        ]
    if case == "untrusted_approval":
        return [
            review(1, "reviewer-one", "APPROVED", association="CONTRIBUTOR"),
            review(2, "reviewer-two", "APPROVED", association="NONE"),
        ]
    if case == "review_page_full":
        return [
            review(index + 1, f"reviewer-{index}", "COMMENTED")
            for index in range(100)
        ]
    return approvals + [
        review(3, "observer-three", "COMMENTED", association="CONTRIBUTOR")
    ]


def workflow_run(
    run_id: int,
    name: str,
    path: str,
    created_at: str,
    *,
    conclusion: str = "success",
    attempt: int = 1,
) -> dict[str, Any]:
    return {
        "id": run_id,
        "name": name,
        "path": f".github/workflows/{path}",
        "event": "pull_request",
        "head_sha": COMMIT,
        "head_branch": "next/android-rish",
        "repository": {"full_name": REPOSITORY},
        "head_repository": {"full_name": REPOSITORY},
        "pull_requests": [{"number": PULL_REQUEST}],
        "run_attempt": attempt,
        "created_at": created_at,
        "run_started_at": created_at,
        "status": "completed",
        "conclusion": conclusion,
    }


def workflow_runs(case: str) -> list[dict[str, Any]]:
    runs = [
        workflow_run(
            11,
            "CI",
            "ci.yml",
            "2026-07-30T10:00:00Z",
            conclusion="failure",
        ),
        workflow_run(
            12,
            "Security",
            "security.yml",
            "2026-07-30T10:00:00Z",
            conclusion="failure",
        ),
        workflow_run(
            13,
            "Android Cross Compile",
            "android-cross-compile.yml",
            "2026-07-30T10:00:00Z",
            conclusion="failure",
        ),
        workflow_run(101, "CI", "ci.yml", "2026-07-30T11:00:00Z"),
        workflow_run(102, "Security", "security.yml", "2026-07-30T11:01:00Z"),
        workflow_run(
            103,
            "Android Cross Compile",
            "android-cross-compile.yml",
            "2026-07-30T11:02:00Z",
        ),
    ]
    if case == "missing_companion":
        return [item for item in runs if item["name"] != "Security"]
    if case == "stale_latest_failure":
        runs.append(
            workflow_run(
                104,
                "CI",
                "ci.yml",
                "2026-07-30T12:00:00Z",
                conclusion="failure",
            )
        )
    if case == "rerun":
        runs.append(
            workflow_run(
                104,
                "CI",
                "ci.yml",
                "2026-07-30T12:00:00Z",
                attempt=2,
            )
        )
    return runs


def main() -> int:
    case = os.environ.get("MOCK_CASE", "pass")
    if len(sys.argv) < 2:
        return 2
    url = sys.argv[-1]

    if f"/pulls/{PULL_REQUEST}/reviews?" in url:
        emit(reviews_for(case))
        return 0
    if url.endswith(f"/pulls/{PULL_REQUEST}"):
        emit(
            {
                "number": PULL_REQUEST,
                "state": "closed" if case == "closed" else "open",
                "draft": case == "draft",
                "user": {"login": "candidate-author"},
                "head": {
                    "sha": OLD_COMMIT if case == "moved" else COMMIT,
                    "ref": "next/android-rish",
                    "repo": {
                        "full_name": (
                            "outside/fork" if case == "fork" else REPOSITORY
                        )
                    },
                },
                "base": {
                    "ref": "develop" if case == "non_main_base" else "main",
                    "repo": {"full_name": REPOSITORY},
                },
            }
        )
        return 0
    if "/actions/runs?" in url:
        runs = workflow_runs(case)
        total = len(runs) + (1 if case == "incomplete_run_page" else 0)
        emit({"total_count": total, "workflow_runs": runs})
        return 0
    if "/actions/runs/" in url:
        try:
            run_id = int(url.rsplit("/", 1)[1])
        except ValueError:
            return 2
        matching = [item for item in workflow_runs(case) if item["id"] == run_id]
        if len(matching) != 1:
            return 22
        emit(matching[0])
        return 0
    return 22


if __name__ == "__main__":
    raise SystemExit(main())
