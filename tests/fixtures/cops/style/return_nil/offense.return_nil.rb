# nitrocop-config: EnforcedStyle: return_nil
def update_state(transition, notice)
  alert = @booth.update_state(transition, notice)

  if alert.blank?
    flash[:notice] = notice
    redirect_back_or_to(admin_conference_booths_path(conference_id: @conference.short_title)) && return
                                                                                                 ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
  else
    flash[:error] = alert
    return redirect_back_or_to(admin_conference_booths_path(conference_id: @conference.short_title)) && return
                                                                                                          ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
  end
end
