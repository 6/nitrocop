items.each { | x | puts x }
items.each do | x |
  puts x
end
shared_examples 'without extra space before multiline closing pipe' do |
  index:,
  other_index:
|
  puts index
end
super do | klass, names, options |
  puts klass
end
super { | x | puts x }
->( x, y ) { puts x }
