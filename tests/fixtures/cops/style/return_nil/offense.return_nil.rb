# nitrocop-config: EnforcedStyle: return_nil
def update_state(value)
  return value && return
                  ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
end
