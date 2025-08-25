import { Sun } from '@/assets/icons/sun';
import { Moon } from '@/assets/icons/moon';

import { useTheme } from '@/components/ThemeProvider';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip';

interface ThemeToggleProps {
  style?: React.CSSProperties;
  iconStyle?: React.CSSProperties;
  hover: boolean;
  spacing: PaddingSpacing;
}

export enum PaddingSpacing {
  SMALL = 'small',
  LARGE = 'large',
}

export function ThemeToggle({ style, hover, iconStyle, spacing }: ThemeToggleProps) {
  const { setTheme, theme } = useTheme();

  return (
    <Tooltip>
      <TooltipTrigger
        className={`${hover ? 'hover:bg-accent' : undefined} float-right mr-2 mt-1 flex fill-current items-center gap-3 rounded-lg ${
          spacing === PaddingSpacing.SMALL ? `px-1 py-1` : `px-3 py-2`
        } text-muted-foreground transition-all hover:fill-yellow-500 cursor-pointer`}
        onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}
        style={style}
      >
        <Sun
          className={`transition-all ${theme === 'light' ? 'opacity-100' : 'opacity-0'} scale-100`}
          style={iconStyle}
        />
        <Moon
          className={`absolute rotate-90 scale-0 transition-all dark:rotate-0 dark:scale-100 ${
            theme === 'dark' ? 'opacity-100' : 'opacity-0'
          }`}
          style={iconStyle}
        />
      </TooltipTrigger>
      {hover && (
        <TooltipContent>
          <p>{theme === 'light' ? 'Dark' : 'Light'} mode</p>
        </TooltipContent>
      )}
    </Tooltip>
  );
}
