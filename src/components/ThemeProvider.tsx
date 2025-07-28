import React, { createContext, useContext, useEffect, useState } from 'react';

export type Theme = 'dark' | 'light';

type ThemeProviderProps = {
  children: React.ReactNode;
  defaultTheme?: Theme;
  storageKey?: string;
  scoped?: boolean;
  globalKey?: string;
};

type ThemeProviderState = {
  theme: Theme;
  setTheme: (theme: Theme) => void;
};

const ThemeProviderContext = createContext<ThemeProviderState | undefined>(undefined);

export function ThemeProvider({
  children,
  defaultTheme = 'light',
  storageKey = 'theme',
  scoped = false,
  globalKey,
}: ThemeProviderProps) {
  const [theme, setThemeState] = useState<Theme>(() => {
    const storedTheme = localStorage.getItem(storageKey);
    return (storedTheme as Theme) || defaultTheme;
  });

  useEffect(() => {
    if (storageKey) {
      localStorage.setItem(storageKey, theme);
    }

    const root = window.document.documentElement;
    if (!scoped) {
      root.classList.remove('light', 'dark');
      root.classList.add(theme);
    }

    if (scoped && theme === 'dark' && globalKey) {
      const global = localStorage.getItem(globalKey);
      if (global === 'dark') {
        localStorage.setItem(storageKey, 'light');
      }
    }
  }, [theme, storageKey, scoped]);

  const setTheme = (newTheme: Theme) => {
    setThemeState(newTheme);
  };

  const value = {
    theme,
    setTheme,
  };

  return (
    <ThemeProviderContext.Provider value={value}>
      {scoped ? <div className={`scoped-theme ${theme}`}>{children}</div> : <>{children}</>}
    </ThemeProviderContext.Provider>
  );
}

export const useTheme = () => {
  const context = useContext(ThemeProviderContext);

  if (!context) {
    throw new Error('useTheme must be used within a ThemeProvider');
  }

  return context;
};
