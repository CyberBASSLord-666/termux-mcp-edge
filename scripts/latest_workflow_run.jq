def workflow_run_timestamp_valid:
  type == "string"
  and test(
    "^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$"
  );

def complete_workflow_run_page:
  if type != "object" then
    error("workflow run page is not an object")
  elif
    (.total_count | type) != "number"
    or .total_count < 0
    or .total_count != (.total_count | floor)
    or .total_count > 100
  then
    error("workflow run page count is invalid")
  elif (.workflow_runs | type) != "array" then
    error("workflow run page has no run array")
  elif .total_count != (.workflow_runs | length) then
    error("workflow run page is incomplete")
  else
    .workflow_runs
  end;

def latest_workflow_run_or_null:
  if type != "array" then
    error("workflow run set is not an array")
  elif any(.[];
    (.id | type) != "number"
    or .id < 1
    or .id != (.id | floor)
    or (.run_attempt | type) != "number"
    or .run_attempt < 1
    or .run_attempt != (.run_attempt | floor)
    or ((.created_at | workflow_run_timestamp_valid) | not)
    or (
      .run_started_at != null
      and ((.run_started_at | workflow_run_timestamp_valid) | not)
    )
  ) then
    error("workflow run identity, attempt, or timestamp is invalid")
  elif ([.[].id] | unique | length) != length then
    error("workflow run IDs are not unique")
  else
    (sort_by(.created_at, .id) | last) as $latest_created
    | (
        sort_by((.run_started_at // .created_at), .run_attempt, .id)
        | last
      ) as $latest_started
    | if $latest_created == null then
        null
      elif $latest_created.id != $latest_started.id then
        error("workflow run ordering is ambiguous")
      else
        $latest_started
      end
  end;

def latest_workflow_run:
  latest_workflow_run_or_null
  | if . == null then
      error("workflow run set is empty")
    else
      .
    end;
