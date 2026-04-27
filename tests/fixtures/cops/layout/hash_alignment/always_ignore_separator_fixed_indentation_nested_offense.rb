# nitrocop-config: EnforcedHashRocketStyle: separator, EnforcedColonStyle: separator, EnforcedLastArgumentHashStyle: always_ignore, ArgumentAlignmentStyle: with_fixed_indentation
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

expect do
  get :show, :params => { :id => 5,
                          :session => "secret_hash",
                          ^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
                          :regexp_param => "ten years" }
                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Layout/HashAlignment: Align the separators of a hash literal if they span more than one line.
end.to raise_error(Apipie::ParamInvalid, /regexp_param/)
