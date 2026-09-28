import type { Locale } from './index';

export const donationCopy = {
  en: {
    eyebrow: 'Support Mouzi', title: 'Help me keep building Mouzi.',
    body: 'Mouzi is free and open source. If you find it useful, you can support development and future updates with a coffee.',
    note: 'Every contribution is optional. Thank you.', button: 'Support Mouzi on Ko-fi',
  },
  pl: {
    eyebrow: 'Wesprzyj Mouzi', title: 'Pomóż mi dalej rozwijać Mouzi.',
    body: 'Mouzi jest darmowe i ma otwarty kod. Jeśli Ci się przydaje, możesz postawić mi kawę i wesprzeć rozwój oraz kolejne aktualizacje.',
    note: 'Wsparcie jest całkowicie dobrowolne. Dziękuję.', button: 'Wesprzyj Mouzi na Ko-fi',
  },
  de: {
    eyebrow: 'Mouzi unterstützen', title: 'Hilf mir, Mouzi weiterzuentwickeln.',
    body: 'Mouzi ist kostenlos und Open Source. Wenn dir die App hilft, kannst du mir einen Kaffee spendieren und damit die Entwicklung und künftige Updates unterstützen.',
    note: 'Jeder Beitrag ist freiwillig. Vielen Dank.', button: 'Mouzi auf Ko-fi unterstützen',
  },
  es: {
    eyebrow: 'Apoya Mouzi', title: 'Ayúdame a seguir desarrollando Mouzi.',
    body: 'Mouzi es gratis y de código abierto. Si te resulta útil, puedes invitarme a un café para apoyar el desarrollo y las próximas actualizaciones.',
    note: 'Todas las aportaciones son voluntarias. Gracias.', button: 'Apoya Mouzi en Ko-fi',
  },
  fr: {
    eyebrow: 'Soutenir Mouzi', title: 'Aidez-moi à continuer à développer Mouzi.',
    body: 'Mouzi est gratuit et open source. Si l’application vous est utile, vous pouvez m’offrir un café pour soutenir son développement et les prochaines mises à jour.',
    note: 'Chaque contribution est facultative. Merci.', button: 'Soutenir Mouzi sur Ko-fi',
  },
  it: {
    eyebrow: 'Sostieni Mouzi', title: 'Aiutami a continuare a sviluppare Mouzi.',
    body: 'Mouzi è gratuito e open source. Se ti è utile, puoi offrirmi un caffè per sostenere lo sviluppo e i prossimi aggiornamenti.',
    note: 'Ogni contributo è facoltativo. Grazie.', button: 'Sostieni Mouzi su Ko-fi',
  },
} satisfies Record<Locale, { eyebrow: string; title: string; body: string; note: string; button: string }>;
