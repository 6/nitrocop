# nitrocop-config: EnforcedStyle: no_space
def switch(options)
  options = options.with_indifferent_access
  else_part = options.delete :else
  "CASE #{options.map do |k, v|
    "\n\t\tWHEN #{field_name_for k}\n\t\t" \
      "THEN #{field_name_for v}"
  end.join(', ')}\n\t\tELSE #{field_name_for else_part}\n\tEND"
end
