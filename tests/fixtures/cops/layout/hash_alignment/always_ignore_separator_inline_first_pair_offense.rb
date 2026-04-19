# nitrocop-config: EnforcedStyle: separator, separator, always_ignore
LAST_SEEN = { daily:  LESS_THAN_A_DAY,
              weekly: LESS_THAN_A_WEEK }
              ^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.

render :show, locals: { user:          current_user,
                        contributions: contributions,
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
                        projects:      projects,
                        ^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
                        gift_form:     gift_form }
                        ^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
