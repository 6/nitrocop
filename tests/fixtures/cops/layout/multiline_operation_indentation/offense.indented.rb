# nitrocop-config: EnforcedStyle: indented

def lyrics
  "#{bottle_number} of beer on the wall, ".capitalize +
  "#{bottle_number} of beer.\n" +
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  "#{bottle_number.action}, " +
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
  "#{bottle_number.successor} of beer on the wall.\n"
  ^ Layout/MultilineOperationIndentation: Use 2 (not 0) spaces for indenting an expression spanning multiple lines.
end

def badge_limit?(user_badge, user_badges)
  if !user_badge.is_favorite &&
       user_badges.select(:badge_id).distinct.where(is_favorite: true).count >=
       ^ Layout/MultilineOperationIndentation: Use 4 (not 5) spaces for indenting a condition in an `if` statement spanning multiple lines.
         SiteSetting.max_favorite_badges
         ^ Layout/MultilineOperationIndentation: Use 4 (not 2) spaces for indenting a condition in an `if` statement spanning multiple lines.
    true
  end
end

def max_chunk_size(uri)
  if uri.host =~
       /(^|\.)amazon\.(com|ca)\z/
       ^ Layout/MultilineOperationIndentation: Use 4 (not 5) spaces for indenting a condition in an `if` statement spanning multiple lines.
    500
  end
end

def null_mail?(message)
  if ActionMailer::Base::NullMail ===
       (
       ^ Layout/MultilineOperationIndentation: Use 4 (not 5) spaces for indenting a condition in an `if` statement spanning multiple lines.
         begin
           message
         rescue StandardError
           nil
         end
       )
    true
  end
end

def reverse_charge_notice(chargeable)
  value = \
    if Compliance::Countries::GST_APPLICABLE_COUNTRY_CODES.include?(chargeable.purchase_sales_tax_info&.country_code) ||
       Compliance::Countries::IND.alpha2 == chargeable.purchase_sales_tax_info&.country_code
       ^ Layout/MultilineOperationIndentation: Use 4 (not 3) spaces for indenting a condition in an `if` statement spanning multiple lines.
      "Reverse Charge"
    end
end
