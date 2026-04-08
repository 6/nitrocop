# nitrocop-config: EnforcedStyle: return_nil
# Mirrors the openSUSE/osem controller pattern from the corpus.
def update_state(alert, notice)
  if alert.blank?
    flash[:notice] = notice
    redirect_back_or_to(admin_conference_booths_path(conference.short_title)) && return
                                                                                 ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
  else
    flash[:error] = alert
    return redirect_back_or_to(admin_conference_booths_path(conference.short_title)) && return
                                                                                        ^^^^^^ Style/ReturnNil: Use `return nil` instead of `return`.
  end
end
