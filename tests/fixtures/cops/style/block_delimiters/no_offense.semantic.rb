# nitrocop-config: EnforcedStyle: semantic

def set(configuration_hash)
  super(configuration_hash.transform_keys { transform_key(_1) })
end

payload = {
  ingredients_with_errors: items.map do |item|
    { id: item.id }
  end
}

if sinks.find do |sink|
     sink.tainted_value == cookie.name ||
       sink.tainted_value == cookie.value
   end
  process
end

def selected_ids(items)
  return items.map { |item|
    item.id
  }
end

def log_error
  do_work
rescue => e
  logger.debug { e.message }
end

assert_equal(*args.map { |str|
  str.tr("\n", "")
})
