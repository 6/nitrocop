# nitrocop-config: EnforcedStyle: for

# Single-line each blocks are always allowed
[1, 2, 3].each { |n| puts n }

# each_with_index is not 'each' - should not be flagged
[1, 2, 3].each_with_index do |n, i|
  puts "#{i}: #{n}"
end

# No receiver - not flagged
each do |n|
  puts n
end

# For loops are accepted when EnforcedStyle: for
for n in [1, 2, 3] do
  puts n
end