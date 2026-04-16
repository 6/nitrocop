# nitrocop-config: EnforcedStyle: return_nil
def update_state
  return redirect_back_or_to(admin_conference_booths_path(conference_id: @conference.short_title)) && return
                                                                                                      ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
end

def update_event_state
  return redirect_back_or_to(admin_conference_program_events_path(conference_id: @conference.short_title)) && return
                                                                                                              ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
end
