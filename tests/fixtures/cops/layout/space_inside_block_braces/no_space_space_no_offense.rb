# nitrocop-config: EnforcedStyle: no_space
# nitrocop-config: EnforcedStyleForEmptyBraces: space
items.map { |item|
  item.do_something
}

foo {[
  bar
]}

app = lambda { |env|
  [200, {'Content-Type'=>'text/plain'}, ['Hello World']] }

scope :find_comments_by_user, lambda { |user|
  where(:user_id => user.id).order('created_at DESC')
}
