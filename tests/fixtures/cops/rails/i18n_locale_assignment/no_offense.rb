I18n.with_locale(:en) { do_something }
I18n.default_locale = :en
locale = :en
I18n.available_locales
config.i18n.default_locale = :en
Pagy::I18n.locale = 'en'
Mongoid::Fields::I18n.locale = nil
Mongoid::Fields::I18n.locale = :fr
