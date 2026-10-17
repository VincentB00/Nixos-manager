import { Directive, ElementRef, inject } from '@angular/core';
import { MatTooltip } from '@angular/material/tooltip';

/** Icon-only buttons rely on matTooltip for their meaning; give them an
 *  accessible name too (screen readers, keyboard users, automation). */
@Directive({ selector: '[matTooltip]', host: { '[attr.aria-label]': 'label()' } })
export class TooltipAriaDirective {
  private tip = inject(MatTooltip);
  private el = inject<ElementRef<HTMLElement>>(ElementRef);
  label(): string | null {
    const el = this.el.nativeElement;
    if (el.getAttribute('aria-label') && el.getAttribute('aria-label') !== this.tip.message) return el.getAttribute('aria-label');
    // elements with visible text already have a name
    const text = (el.textContent ?? '').replace(/\s+/g, ' ').trim();
    const icons = Array.from(el.querySelectorAll('mat-icon')).map(i => (i.textContent ?? '').trim());
    const visible = icons.reduce((t, i) => t.replace(i, ''), text).trim();
    return visible ? null : (this.tip.message || null);
  }
}
