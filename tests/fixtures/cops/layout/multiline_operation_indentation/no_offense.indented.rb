# nitrocop-config: EnforcedStyle: indented

def lyrics
  "#{bottle_number} of beer on the wall, ".capitalize +
    "#{bottle_number} of beer.\n" +
    "#{bottle_number.action}, " +
    "#{bottle_number.successor} of beer on the wall.\n"
end

def purchase_with_sales_tax_info_as_chargeable
  @_purchase_with_sales_tax_info_as_chargeable ||= \
    successful_purchases.find { _1.purchase_sales_tax_info&.business_vat_id.present? } ||
    purchase_with_tax_as_chargeable
end
