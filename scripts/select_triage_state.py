"""Select the last completed state owner, never falling back past missing state."""
import json
import os
import subprocess
from urllib.parse import quote


def api_pages(endpoint):
    return json.loads(subprocess.check_output(
        ["gh", "api", "--paginate", "--slurp", endpoint], text=True
    ))


def select_run(runs, current_id, jobs_for):
    candidates = []
    for run in runs:
        if run["id"] == current_id:
            continue
        # Pending requests cannot yet have changed state under workflow concurrency.
        if (run.get("run_attempt", 1) == 1
                and run["status"] in ("queued", "requested", "waiting", "pending")):
            continue
        jobs = jobs_for(run["id"])
        if any("run_attempt" not in job for job in jobs):
            raise RuntimeError("Cannot establish job attempt safely")
        # Only attempt 1 may write state. Later attempts are rejected before
        # restoration; their completion timestamps must not reorder old artifacts.
        jobs = [job for job in jobs if job["run_attempt"] == 1]
        started = [
            job for job in jobs
            if any(step.get("started_at") and step.get("conclusion") != "skipped"
                   for step in job.get("steps", []))
        ]
        if run["conclusion"] == "cancelled" and not started:
            continue
        if run["status"] != "completed" or not started:
            raise RuntimeError("Cannot establish the previous run's state safely")
        for job in started:
            if not job.get("completed_at"):
                raise RuntimeError("Previous job has not completed")
        completed = max(job["completed_at"] for job in started)
        candidates.append((completed, run["id"]))
    if not candidates:
        return None, False
    # Completion time follows actual serialized execution, not dispatch IDs.
    candidates.sort(reverse=True)
    if len(candidates) > 1 and candidates[0][0] == candidates[1][0]:
        raise RuntimeError("Ambiguous state order; inspect previous runs")
    previous = candidates[0][1]
    stale = any(run_id > current_id for _, run_id in candidates)
    return previous, stale


def main():
    repo = os.environ["GITHUB_REPOSITORY"]
    branch = quote(os.environ["GITHUB_REF_NAME"], safe="")
    pages = api_pages(
        f"repos/{repo}/actions/workflows/review-triage.yml/runs"
        f"?branch={branch}&event=workflow_dispatch&per_page=100"
    )
    runs = [run for page in pages for run in page["workflow_runs"]]

    def jobs_for(run_id):
        # Include all attempts so a rejected rerun cannot hide an earlier write.
        return [job for page in api_pages(
            f"repos/{repo}/actions/runs/{run_id}/jobs?filter=all&per_page=100"
        ) for job in page["jobs"]]

    previous, stale = select_run(runs, int(os.environ["GITHUB_RUN_ID"]), jobs_for)
    print(f"{previous or ''}|{'true' if stale else 'false'}")


if __name__ == "__main__":
    main()
