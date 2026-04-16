# nitrocop-config: EnforcedStyle: return_nil

def update_booth_state(path)
  return redirect_back_or_to(path) && return
                                      ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
end

def update_event_state(path)
  return redirect_back_or_to(path) && return
                                      ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
end
