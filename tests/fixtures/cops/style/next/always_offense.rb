# nitrocop-config: EnforcedStyle: always

items.each do |from|
  raise(
  ^^^^^ Style/Next: Use `next` to skip iteration.
    MyError,
    :file => from
  ) unless File.exist?(from)
end

items.each do |item|
  work do
  ^^^^^^^ Style/Next: Use `next` to skip iteration.
    step_one
    step_two
  end if condition
end
