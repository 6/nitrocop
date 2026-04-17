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

# ternary with assignment whose correction would exceed line length should not be flagged
(t.empty? || t == "y" || t == "yes" || t == "yeah") ? conf["env"]["testmode_enabled"] = true : conf["env"]["testmode_enabled"] = false

# FP: safe-navigation comparisons are csend in RuboCop and should not be flagged
def custom_start?
  increment_by&.<(0) ? start_with&.!=(max_value) : start_with&.!=(min_value)
end

# Default config: line-length guard suppresses this offense, but it returns when
# Layout/LineLength is disabled in repo config.
module Jetpants
  class Pool
    def to_hash(for_app_config = false)
      if for_app_config
        slave_data = active_slave_weights.map { |db, weight| { 'host' => db.to_s, 'weight' => weight } }
      else
        slave_data = active_slave_weights.map { |db, weight| { 'host' => db.to_s, 'weight' => weight, 'role' => 'ACTIVE_SLAVE' } } +
                     standby_slaves.map { |db| { 'host' => db.to_s, 'role' => 'STANDBY_SLAVE' } } +
                     backup_slaves.map { |db| { 'host' => db.to_s, 'role' => 'BACKUP_SLAVE' } }
      end
      slave_data
    end
  end
end
