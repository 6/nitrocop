# nitrocop-config: EnforcedStyle: separator, separator, always_ignore
hash = {
  stack_id:,
  name: value
}

create_or_update_by!(
  selector: {
    github_id: check_run.id
  },
  attributes: {
    stack_id:,
    name: check_run.name,
    conclusion: check_run.conclusion
  }
)
