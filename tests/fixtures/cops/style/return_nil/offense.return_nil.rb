# nitrocop-config: EnforcedStyle: return_nil
def update_state
  return redirect_back_or_to(path) && return
                                      ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
end
