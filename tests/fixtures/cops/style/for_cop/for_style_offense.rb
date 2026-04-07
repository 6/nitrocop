# nitrocop-config: EnforcedStyle: for
[1, 2, 3].each do |n|
^^^^^^^^^^^^^^^^^^^^^^ Style/For: Prefer `for` over `each`.
  puts n
end

items.each do |x|
^^^^^^^^^^^^^^^^^^^ Style/For: Prefer `for` over `each`.
  process(x)
end

(1..10).each do |i|
^^^^^^^^^^^^^^^^^^^^ Style/For: Prefer `for` over `each`.
  puts i
end