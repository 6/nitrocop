# nitrocop-config: EnforcedStyle: always
# Bare named param as entire body — RuboCop's each_descendant skips the body node itself
block { |n| n }
-> { |n| n }
lambda { |x| x }
proc { |x| x }
# Multi-line bare named param
block do
  |n|
  n
end
