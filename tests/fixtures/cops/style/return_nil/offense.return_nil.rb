# nitrocop-config: EnforcedStyle: return_nil
def update_state(alert)
  if alert.blank?
    redirect_back_or_to(path) && return
                                 ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
  else
    return redirect_back_or_to(path) && return
                                        ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
  end
end
