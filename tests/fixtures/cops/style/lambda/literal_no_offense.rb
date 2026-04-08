# RuboCop's parser crashes on this escape sequence and never reaches Style/Lambda.
assert_equal(0, /\c\xFF/ =~ "\c\xFF")
worker = lambda do
  do_work
end
