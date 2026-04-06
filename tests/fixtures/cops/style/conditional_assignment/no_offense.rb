x = if condition
  1
else
  2
end

if condition
  x = 1
else
  y = 2
end

if condition
  do_something
else
  do_other_thing
end

# case without else should not be flagged
case x
when 1
  y = 1
when 2
  y = 2
end

# case where branches assign different variables
case x
when 1
  y = 1
when 2
  z = 2
else
  w = 3
end

# case where branches have different assignment types
case x
when 1
  y = 1
else
  do_something
end

# if/else with index setter assigning to different keys should not be flagged
if result.success?
  flash[:notice] = "Success"
else
  flash[:error] = "Failed"
end

# case/when with index setter assigning to different keys
case action
when :create
  flash[:success] = "Created"
when :update
  flash[:notice] = "Updated"
else
  flash[:error] = "Failed"
end

# if/else with correction exceeding line length should not be flagged
if ActionView::Base.respond_to?(:with_empty_template_cache) && ActionView::Base.respond_to?(:with_view_paths)
  @apipie_renderer = ActionView::Base.with_empty_template_cache.with_view_paths(base_paths + layouts_paths)
else
  @apipie_renderer = ActionView::Base.new(base_paths + layouts_paths)
end

# if/else with shovel operator assigning to different receivers should not be flagged
if condition
  out << 1
else
  other << 2
end

# if/else with setter ||= assigning to different methods should not be flagged
if condition
  self.foo ||= 1
else
  self.bar ||= 2
end

# if/else with index += assigning to different keys should not be flagged
if condition
  totals[:a] += 1
else
  totals[:b] += 2
end

# if/else with comparison sends on different receivers should not be flagged
if condition
  match.should == true
else
  other.should == false
end
