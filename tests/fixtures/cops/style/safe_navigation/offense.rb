foo && foo.bar
^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

foo && foo.bar(param1, param2)
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

foo && foo.bar.baz
^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

foo && foo.nil?
^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

foo.nil? ? nil : foo.bar
^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

foo ? foo.bar : nil
^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

!foo.nil? ? foo.bar : nil
^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

callback.call unless callback.nil?
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

handler.process unless handler.nil?
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

obj.bar if obj
^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if data
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  data.each do |k, v|
  end
end

after_save { if user then user.update_contribution_count end }
             ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

after_destroy { if user then user.update_contribution_count end }
                ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

return self[:postmark_template_alias] && self[:postmark_template_alias].to_s if val.nil?
       ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

mail.cc && (mail.cc.include? 'support@agileventures.org')
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if fd && fd.respond_to?(:each)
   ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  fd.each {}
end

if new_model_collection and new_model_collection.is_a?(Array)
   ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  new_model_collection.each {}
end

(other.class == Path) && geometry.equals(other && other.respond_to?(:geometry) && other.geometry)
                                         ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

proof && dom_body && dom_body.include?( proof )
         ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

proof && response && response.include?( proof )
         ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

uri.port = port ? port.to_i : nil
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

uri.port = port ? port.to_i : nil
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

session[taxonomy_id] = taxonomy ? taxonomy.id : nil
                       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

user_is_current_user && record.campaign && record.campaign.users_can_join?
                        ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

user_is_current_user && record.campaign && record.campaign.dms_can_join?
                        ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

errors && errors.is_a?(Array) || errors.is_a?(String)
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

errors && (errors.is_a?(Array) && errors != EMPTY_ARRAY) || (errors.is_a?(String))
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

cond && @chunks[0] && @chunks[0].is_a?(String)
        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

BTC::Invariant(foo.nil? ? nil : foo.to_s, "message")
               ^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

ActiveRecord::Migrator.migrate(Skyline.root + "db/migrate/", ENV["VERSION"] ? ENV["VERSION"].to_i : nil)
                                                             ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

a && a.b && c && c.d
^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
            ^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if e && e.message
   ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

foo && (foo.bar)
^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

Outer.a(Inner.b(foo ? foo.bar : nil))
                ^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

loop do
  if e && e.message
     ^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
    response[:what] = e.message
  end
end

items.collect do |klass|
  param = @parameters[klass.name]
  param.to_edges if param
  ^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
end.flatten.compact

values.__send__(:each) do |value|
  value.foo if value
  ^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
end

[zip64 && zip64.compressed_size ? 0xFFFFFFFF : @compressed_size].pack("V")
 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if handler && (!handler.is_a?(Proc) || (handler.lambda? && handler.arity != 0))
   ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if handler && (!handler.is_a?(Proc) || (handler.lambda? && handler.arity != 1))
   ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if handler && (!handler.is_a?(Proc) || (handler.lambda? && handler.arity != 2))
   ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

foo&.bar && foo.bar.baz
^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

foo && (foo.bar? || (foo.baz? && foo.quux?))
^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

(foo && foo.bar).custom_method
 ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

(foo ? foo.bar : nil).custom_method
 ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

(foo.bar if foo).custom_method
 ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

render json: collection.select { |c| c.matrix && c.matrix.flatten.index(sample.id) }
                                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

config.paperclip_defaults = if ENV['S3_SERVICE'] && ENV['S3_SERVICE'].casecmp('AWS').zero?
                               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  {}
else
  {}
end

definitions << { 'id' => attr['id'] ? attr['id'].to_i : nil }
                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

UIView.animateWithDuration(1, completion: lambda {
  block.call if block
  ^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
})

obj.send(:x, hash["step"] && hash["step"].downcase.to_sym)
             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

block_end.nil? ? nil : block_end.offset))
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

format_elements(@elements, indent ? indent.increase : nil, b)
                           ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

format_elements(@elements, indent ? indent.increase : nil, b) unless @elements.empty?
                           ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

raw_response = parse(ssl_request(method, live_url + url, (parameters ? parameters.to_query : nil), headers(options)))
                                                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

raw_response = ssl_request(method, url, (parameters ? parameters.to_json : nil), headers(options))
                                         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

after_commit -> { container.cards.touch_all if container }
                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

do_parse_json(options, (record && record.orm_model.schema) || (model && model.schema), record)
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

def penalize!(amount=(((team && team.members.size) || 6) / 6.0)*activitiy_multipler)
                        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if opponent_names then opponent_names.split(' ') end
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if number_of_hands then number_of_hands.to_i end
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

text_field_tag("#{name}[value]", (value ? value.to_d : nil))
                                  ^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

Packet.from_source(self, (session.event_type if session))
                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

text_field_tag("#{name}[value]", (value ? value.to_d : nil)) +
                                  ^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  text_field_tag("#{name2}[value]", (value2 ? value2.to_d : nil))
                                     ^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

Net::IMAP.recordings[digest] << [action, response.dup, @responses ? @responses.dup : nil, all_responses]
                                                       ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

pages << ($1.nil?? nil : $1.to_i)
          ^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

methods-[extension_modules.last ? extension_modules.last.methods : nil]-self.class.instance_methods
         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

((product && product.population_counting.to_sym) == expected ? 1 : 0)
  ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

if lat_lng && !(lat_lng.first.zero? && lat_lng.last.zero?)
   ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  lat_lng
end

if data && data.notnull.any?
   ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  data
end

options && options.each do |option, value|
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  option
end

parts[:scheme] = (scheme.dup.force_encoding(Encoding::UTF_8) if scheme)
                  ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

parts[:authority] = (authority.dup.force_encoding(Encoding::UTF_8) if authority)
                     ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

outer.wrap do
  inner.wrap do
    items.each do |item|
    ^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
      item
    end if items
  end
end

options && options.each do |option, value|
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  option
end

new(
  input_attributes: {
    error: (errors.to_sentence.capitalize if errors),
            ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
  }
)

component("ui/forms/select").new(
  error: (errors.to_sentence.capitalize if errors),
          ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
)

a && a.b && c && c.d && e && (f && f.g)
^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
            ^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
                              ^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

@with.map do |mapping|
  options && options.each do |option, value|
  ^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
    option
  end
end.join(", ")

options && options.each do |option, value|
^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.

@errors = Array(root && root.errors.map(&:to_s))
                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Style/SafeNavigation: Use safe navigation (`&.`) instead of checking if an object exists before calling the method.
