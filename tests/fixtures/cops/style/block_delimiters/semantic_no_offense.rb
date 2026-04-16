render json: {
  items_with_errors: items.map do |item|
    {
      id: item.id
    }
  end,
  names: items.map { |item|
    item.name
  }
}, status: 422

def set(config)
  super(config.transform_keys { transform_key(_1) })
end

def fallback(config)
  super(config.transform_keys do |key|
    transform_key(key)
  end)
end

return if items.any? do |item|
  item.valid?
end

if items.any? { |item|
     item.valid?
   }
  process
end
