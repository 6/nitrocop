if condition
^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  x = 1
else
  x = 2
end

if foo
^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  bar = something
else
  bar = other_thing
end

if test
^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  result = :yes
else
  result = :no
end

# case/when with local variable assignment
case pwn_provider
^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
when 'aws'
  config_path = 'aws.yaml'
when 'virtualbox'
  config_path = 'vbox.yaml'
else
  config_path = ''
end

# if/else with setter method assignment
if vagrant_gui == 'true'
^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  vm.gui = true
else
  vm.gui = false
end

# if/else with index setter assignment
if name.match?('.xlsx')
^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  mail.attachments[name] = { content: body, transfer_encoding: :base64 }
else
  mail.attachments[name] = body
end

# case/when with setter method assignment
case level.to_s.downcase.to_sym
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
when :debug
  logger.level = Logger::DEBUG
when :error
  logger.level = Logger::ERROR
else
  logger.level = Logger::INFO
end

# case/when with instance variable assignment
case cmd_resp
^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
when '21'
  @msg = :invalid_command
when '28'
  @msg = :card_speed_measurement_start
else
  @msg = :unknown
end

# ternary with local variable assignment
opts[:encoding].nil? ? encoding = nil : encoding = opts[:encoding].to_s
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.

# ternary with setter method assignment
pi.config.pwn_ai_debug ? pi.config.pwn_ai_debug = false : pi.config.pwn_ai_debug = true
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.

# if/elsif/else with same variable assignment
if RUBY_ENGINE == 'ruby'
^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  platform = 'ruby'
elsif RUBY_ENGINE == 'jruby'
  platform = 'java'
else
  platform = 'other'
end

# unless/else with same variable assignment
unless condition
^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  x = 1
else
  x = 2
end

# if/else where both branches assign the same variable (complex condition)
if content_type =~ /json/i && (response_body.is_a?(Hash) || response_body.is_a?(Array))
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  response_body = JSON.generate(response_body)
else
  response_body = response_body.to_s
end

# if/else with shovel operator assignment
if @params.empty?
^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  message << " without parameters."
else
  message << " with parameters #{@params.inspect}."
end

# if/else with shovel operator assignment in a block
if i == 0
^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  this_sig_lines << options.indented(indent_level, definition)
else
  this_sig_lines << options.indented(indent_level, "#{' ' * (definition.length - 2)}| ")
end

# if/else with setter ||= assignment
if current_edit_mode == :adapter
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  e.use_key ||= UseInfo.fetch(:adapter).key
else
  e.use_key ||= UseInfo.fetch(:basic).key
end

# if/else with setter += assignment
if type.nil? && vmid.nil?
^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  uri.path += "/nodes/#{node}/vncwebsocket"
else
  uri.path += "/nodes/#{node}/#{type}/#{vmid}/vncwebsocket"
end

# if/else with index ||= assignment
if (klass = Watir.tag_to_class[tag_name])
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  @supported_element_attributes_for[tag_name] ||= Set.new(klass.attribute_list)
else
  @supported_element_attributes_for[tag_name] ||= Set.new
end

# if/else with index += assignment
if o.class == String
^^^^^^^^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  object_space[o.class][:memsize] += o.bytesize
else
  object_space[o.class][:memsize] += ObjectSpace.memsize_of(o)
end

# if/else with comparison sends on same receiver
if (!boolean)
^^^^^^^^^^^^^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
  match.should == true
else
  match.should == false
end

# ternary with local variable assignment (FN case)
opts[:response_timeout].nil? ? response_timeout = 0.9 : response_timeout = opts[:response_timeout].to_f
^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.

# FN: nested if/elsif/else with shovel assignment should ignore branch indentation
module ApplicationHelper
  def custom_pagy_nav(pagy)
    html = '<div class="flex items-center gap-2">'
    pagy.series.each do |item|
      if item.to_s == pagy.page.to_s
      ^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
        html << link_to(item, url_for(page: item), class: "btn btn-sm btn-neutral min-w-[2.5rem] pointer-events-none")
      elsif item.to_s == "gap"
        html << link_to("…", url_for(page: item), class: "btn btn-sm btn-ghost min-w-[2.5rem] pointer-events-none btn-disabled")
      else
        html << link_to(item, url_for(page: item), class: "btn btn-sm btn-ghost min-w-[2.5rem]")
      end
    end
  end
end

# FN: nested setter assignment should ignore branch indentation in line-length checks
module MassiveRecord
  module Adapters
    module Thrift
      class Scanner
        def open
          if created_at.empty?
          ^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
            self.opened_scanner = connection.scannerOpen(table_name, key, formatted_column_family_names, {})
          else
            self.opened_scanner = connection.scannerOpenTs(table_name, key, formatted_column_family_names, created_at, {})
          end
        end
      end
    end
  end
end

# FN: nested index setter assignment inside an outer else branch should still be flagged
class Skyline::PublishedPublicationsController < Skyline::ApplicationController
  def destroy
    if @article.depublishable?
      @article.depublish
      messages[:success] = t(:success, :scope => [:published_publication, @article.class, :destroy, :flashes])
    else
      if @article.persistent?
      ^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
        notifications[:error] = t(:error_persistent, :scope => [:published_publication, @article.class, :destroy, :flashes])
      else
        notifications[:error] = t(:error, :scope => [:published_publication, @article.class, :destroy, :flashes])
      end
    end
  end
end

# FN: nested local variable assignment with a long RHS should ignore branch indentation
module ScumblrTask
  class Base
    def create_event(event, level = "Error")
      if(event.respond_to?(:message))
      ^ Style/ConditionalAssignment: Use the return of the conditional for variable assignment and comparison.
        details = "An error occurred in #{self.class.task_type_name}. Error: #{event.try(:message)}\n\n#{event.try(:backtrace)}"
      else
        details = event.to_s
      end
    end
  end
end
